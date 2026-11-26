use super::function_types::expressions::{assertion, parent};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::span::GetSpan;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-chained-type-assertions",
    family: Family::Types,
    gate: Gate::Blocking,
    provenance: "Port of anti-slop no-chained-type-assertions.",
    fix: "Keep the original precise type, or parse untrusted input before narrowing it.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    for (id, node) in ctx.nodes_with_ids() {
        let Some((mut expression, ty)) = assertion(node.kind()) else {
            continue;
        };
        if parent(ctx, id).and_then(assertion).is_some() {
            continue;
        }
        let mut only_const = ty.is_const_type_reference();
        while let Some((inner, ty)) =
            assertion(nodes.kind(expression.without_parentheses().node_id()))
        {
            only_const &= ty.is_const_type_reference();
            if !only_const {
                findings.report(node.span().start as usize, None);
                break;
            }
            expression = inner;
        }
    }
}
