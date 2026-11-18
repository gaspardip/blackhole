use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-runtime-typeof",
    family: Family::Types,
    provenance: "Port of anti-slop no-runtime-typeof.",
    fix: "Parse input at its I/O boundary and branch on the domain value.",
    run: Run::Ts(check),
};

use oxc::{
    ast::{AstKind, ast::Expression},
    span::GetSpan,
    syntax::operator::UnaryOperator,
};
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    for (id, node) in nodes.iter_enumerated() {
        let AstKind::UnaryExpression(n) = node.kind() else {
            continue;
        };
        if n.operator != UnaryOperator::Typeof {
            continue;
        }
        if let Some(AstKind::BinaryExpression(b)) =
            super::function_types::expressions::parent(ctx, id)
            && b.operator.is_equality()
        {
            let other = if b.left.without_parentheses().span() == n.span {
                &b.right
            } else {
                &b.left
            };
            if matches!(other.without_parentheses(),Expression::StringLiteral(s) if s.value=="undefined")
            {
                continue;
            }
        }
        findings.report(n.span.start as usize, None);
    }
}
