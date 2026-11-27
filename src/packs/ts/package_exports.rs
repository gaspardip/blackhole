use crate::engine::{Family, Gate, Rule, Run, StructureCtx, package_dir};
use serde_json::Value;
use std::collections::BTreeSet;

pub const RULE: Rule = Rule {
    id: "ts/package-exports",
    family: Family::Structure,
    gate: Gate::Blocking,
    provenance: "landmines: package-exports",
    fix: "Declare explicit entrypoints or named public namespaces in a valid package.json.",
    run: Run::Structure(check),
};

fn unrestricted(ctx: &StructureCtx<'_>, target: &str) -> bool {
    ctx.record_work(target.len());
    if target.starts_with('/') || target.ends_with('/') {
        return false;
    }
    let mut parts = Vec::new();
    for part in target.split('/') {
        match part {
            "." | "" => (),
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let ([name] | ["src", name]) = parts.as_slice() else {
        return false;
    };
    *name == "*" || name.strip_prefix("*.").is_some_and(super::script_extension)
}

fn problem(ctx: &StructureCtx<'_>) -> Option<String> {
    ctx.record_work(ctx.bytes.len());
    let Ok(Value::Object(manifest)) = serde_json::from_slice(ctx.bytes) else {
        return Some("package.json must contain a valid JSON object".into());
    };
    let mut targets = BTreeSet::new();
    let mut pending = vec![manifest.get("exports")?];
    while let Some(value) = pending.pop() {
        ctx.record_work(1);
        match value {
            Value::String(s) if unrestricted(ctx, s) => {
                targets.insert(s.as_str());
            }
            Value::Object(map) => pending.extend(map.values()),
            Value::Array(array) => pending.extend(array),
            _ => (),
        }
    }
    (!targets.is_empty()).then(|| {
        format!(
            "unrestricted source exports: {}",
            targets.into_iter().collect::<Vec<_>>().join(", ")
        )
    })
}

fn check(ctx: &StructureCtx<'_>, notes: &mut Vec<String>) {
    if package_dir(ctx.path).is_some() {
        notes.extend(problem(ctx));
    }
}
