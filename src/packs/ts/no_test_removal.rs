use super::test_context::test_file_path;
use crate::engine::{self, DiffCtx, DiffFindings, Family, Finding, Gate, Rule, Run};
use std::io;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-test-removal",
    family: Family::Tests,
    gate: Gate::Blocking,
    provenance: "Agents delete failing tests or their assertions and report green.",
    fix: "Put the test back and make it pass. If the behaviour it covered is gone on purpose, or a test moved between files, a person approves the removal.",
    run: Run::Diff(check),
};

fn finding(path: &str, note: String) -> Finding {
    Finding {
        rule: RULE.id,
        gate: RULE.gate,
        path: path.to_owned(),
        line: 1,
        col: 1,
        note: Some(note),
    }
}

fn check(ctx: &mut DiffCtx<'_>, findings: &mut DiffFindings) -> io::Result<()> {
    for old in ctx
        .base
        .paths()
        .filter(|p| test_file_path(p) && !ctx.base_excludes.is_match(p))
    {
        let new = ctx.renames.get(old).copied().unwrap_or(old);
        let old_kind = engine::source_type(old);
        let new_kind = engine::source_type(new);
        let diagnose = old != new && old_kind.is_some() && new_kind.is_none();
        if ctx.approved && !diagnose {
            continue;
        }
        let before = if ctx.approved {
            None
        } else {
            ctx.base.historical_blob(old, ctx.batch)?
        };
        if before.is_none() && !diagnose {
            continue;
        }
        let after = if ctx.head_excludes.is_match(new) {
            None
        } else {
            ctx.head.blob(new, ctx.batch)?
        };
        let Some(after) = after else {
            if before.is_some() {
                findings.removals.push(finding(old, "deleted".into()));
            }
            continue;
        };
        let Some(kind) = old_kind else { continue };
        let resolved = new_kind.unwrap_or(kind);
        let changed = before.is_some_and(|id| id != after || kind != resolved);
        if !changed && !diagnose {
            continue;
        }
        let before = before
            .filter(|_| changed)
            .map(|id| ctx.batch.read(id))
            .transpose()?;
        let after = ctx.batch.read(after)?;
        let count = |ts: &engine::TsCtx<'_>| super::no_assertionless_tests::inventory(ts).counts();
        let (removal, diagnostics) = engine::parser_thread(|| {
            let (head, diagnostics) = engine::test_counts(new, &after, resolved, |ts| {
                before.as_ref().map_or((0, 0), |_| count(ts))
            });
            let removal = before.as_deref().and_then(|bytes| {
                let (base, _) = engine::test_counts(old, bytes, kind, count);
                compare(new, base, head)
            });
            (removal, diagnostics)
        })?
        .map_err(|_| {
            io::Error::other(format!("Parser panicked while counting tests in {new:?}"))
        })?;
        findings.removals.extend(removal);
        if diagnose {
            findings.diagnostics.extend(diagnostics);
        }
    }
    Ok(())
}

fn compare(
    path: &str,
    (cases_before, assertions_before): (usize, usize),
    (cases_after, assertions_after): (usize, usize),
) -> Option<Finding> {
    let notes = [
        ("test cases", cases_before, cases_after),
        ("assertions", assertions_before, assertions_after),
    ]
    .into_iter()
    .filter(|&(_, from, to)| to < from)
    .map(|(label, from, to)| format!("{label} {from} -> {to}"))
    .collect::<Vec<_>>();
    (!notes.is_empty()).then(|| finding(path, notes.join(", ")))
}
