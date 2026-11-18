use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unknown-returns",
    family: Family::Types,
    provenance: "Port of anti-slop no-unknown-returns.",
    fix: "Parse the value at its boundary and return a named domain type.",
    run: Run::Ts(check),
};

use super::{dictionary_types::unknown_return, function_types::returns};
use oxc::span::GetSpan;
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for n in ctx.semantic.nodes().iter() {
        if let Some(a) = returns(n.kind())
            && unknown_return(ctx, &a.type_annotation)
        {
            findings.report(a.type_annotation.span().start as usize, None);
        }
    }
}
