use super::{dictionary_types::Resolution, function_types::returns};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::span::GetSpan;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unknown-returns",
    family: Family::Types,
    provenance: "Port of anti-slop no-unknown-returns.",
    fix: "Parse the value at its boundary and return a named domain type.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let resolution = Resolution::default();
    for n in ctx.semantic.nodes().iter() {
        if let Some(a) = returns(n.kind())
            && resolution.unknown_return(ctx, &a.type_annotation)
        {
            findings.report(
                a.type_annotation.without_parenthesized().span().start as usize,
                None,
            );
        }
    }
}
