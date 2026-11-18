use super::dictionary_types::{Resolution, report};
use super::function_types::{parameter, predicate_subject, returns};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::span::GetSpan;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unknown-parameters",
    family: Family::Types,
    provenance: "Port of anti-slop no-unknown-parameters.",
    fix: "Accept a named domain type. Parse input at the I/O boundary before calling this function.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let resolution = Resolution::default();
    let nodes = ctx.semantic.nodes();
    for (id, n) in nodes.iter_enumerated() {
        resolution.begin_candidate();
        let Some((name, ty)) = parameter(n.kind()) else {
            continue;
        };
        let unknown = resolution.contains_unknown(ctx, ty);
        if matches!(unknown, Ok(false)) || name == Some("cause") {
            continue;
        }
        let owner = nodes.parent_kind(nodes.parent_id(id));
        if returns(owner)
            .and_then(predicate_subject)
            .is_some_and(|subject| name.is_some_and(|n| n == subject))
        {
            continue;
        }
        report(findings, ty.span().start as usize, unknown);
    }
}
