use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-object-parameters",
    family: Family::Types,
    provenance: "Port of anti-slop no-object-parameters.",
    fix: "Accept a named owner type and parse external input before calling this function.",
    run: Run::Ts(check),
};

use super::{dictionary_types::object_input, function_types::parameter};
use oxc::span::GetSpan;
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for node in ctx.semantic.nodes().iter() {
        if let Some((_, ty)) = parameter(node.kind())
            && object_input(ctx, ty)
        {
            findings.report(ty.span().start as usize, None);
        }
    }
}
