use crate::engine::{Findings, TsCtx};
use oxc::ast::{AstKind, ast::Expression};

pub(super) fn check(ctx: &TsCtx<'_>, findings: &mut Findings, method: &str) {
    for n in ctx.semantic.nodes().iter() {
        let AstKind::CallExpression(call) = n.kind() else {
            continue;
        };
        let object = match call.callee.without_parentheses() {
            Expression::StaticMemberExpression(m) if m.property.name == method => &m.object,
            Expression::ComputedMemberExpression(m) if matches!(m.expression.without_parentheses(),Expression::StringLiteral(s) if s.value==method) => {
                &m.object
            }
            _ => continue,
        };
        if let Expression::Identifier(id) = object.without_parentheses()
            && id.name == "Reflect"
            && id.reference_id.get().is_some_and(|r| {
                ctx.semantic
                    .scoping()
                    .get_reference(r)
                    .symbol_id()
                    .is_none()
            })
        {
            findings.report(call.span.start as usize, None);
        }
    }
}
