use crate::engine::{Findings, TsCtx};
use oxc::ast::{AstKind, ast::Expression};
use oxc::semantic::IsGlobalReference;

fn string_key(expr: &Expression<'_>, method: &str) -> bool {
    matches!(expr.without_parentheses(),Expression::StringLiteral(s) if s.value==method)
}

pub(super) fn check(ctx: &TsCtx<'_>, findings: &mut Findings, method: &str) {
    for n in ctx.semantic.nodes().iter() {
        let AstKind::CallExpression(call) = n.kind() else {
            continue;
        };
        let object = match call.callee.without_parentheses() {
            Expression::StaticMemberExpression(m) if m.property.name == method => &m.object,
            Expression::ComputedMemberExpression(m) if string_key(&m.expression, method) => {
                &m.object
            }
            _ => continue,
        };
        if let Expression::Identifier(id) = object.without_parentheses()
            && id.name == "Reflect"
            && id.is_global_reference(ctx.semantic.scoping())
        {
            findings.report(call.span.start as usize, None);
        }
    }
}
