use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, TSType},
    },
    syntax::node::NodeId,
};

pub(crate) fn assertion(kind: AstKind<'_>) -> Option<(&Expression<'_>, &TSType<'_>)> {
    match kind {
        AstKind::TSAsExpression(n) => Some((&n.expression, &n.type_annotation)),
        AstKind::TSTypeAssertion(n) => Some((&n.expression, &n.type_annotation)),
        _ => None,
    }
}
pub(crate) fn parent<'a>(ctx: &TsCtx<'a>, id: NodeId) -> Option<AstKind<'a>> {
    let nodes = ctx.semantic.nodes();
    nodes
        .ancestor_ids(id)
        .map(|p| nodes.kind(p))
        .find(|k| !matches!(k, AstKind::ParenthesizedExpression(_)))
}
pub(crate) fn known_value(expr: &Expression<'_>) -> bool {
    expr.is_literal()
        || matches!(
            expr,
            Expression::TemplateLiteral(_)
                | Expression::ArrayExpression(_)
                | Expression::ArrowFunctionExpression(_)
                | Expression::ClassExpression(_)
                | Expression::FunctionExpression(_)
                | Expression::NewExpression(_)
                | Expression::ObjectExpression(_)
        )
}
