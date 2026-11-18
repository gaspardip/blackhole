use super::{
    dictionary_types::{Resolution, report},
    function_types::parameter,
};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::span::GetSpan;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-object-parameters",
    family: Family::Types,
    provenance: "Port of anti-slop no-object-parameters.",
    fix: "Accept a named owner type and parse external input before calling this function.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let resolution = Resolution::default();
    for node in ctx.semantic.nodes().iter() {
        if let Some((_, ty)) = parameter(node.kind()) {
            report(
                findings,
                ty.span().start as usize,
                resolution.object_input(ctx, ty),
            );
        }
    }
}
