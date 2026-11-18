use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-chained-type-assertions",
    family: Family::Types,
    provenance: "Port of anti-slop no-chained-type-assertions.",
    fix: "Keep the original precise type, or parse untrusted input before narrowing it.",
    run: Run::Ts(check),
};

use oxc::ast::{AstKind, ast::Expression};
use oxc::span::GetSpan;

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    for (id, node) in nodes.iter_enumerated() {
        let (mut expression, mut only_const) = match node.kind() {
            AstKind::TSAsExpression(n) => {
                (&n.expression, n.type_annotation.is_const_type_reference())
            }
            AstKind::TSTypeAssertion(n) => {
                (&n.expression, n.type_annotation.is_const_type_reference())
            }
            _ => continue,
        };
        if matches!(
            nodes
                .ancestor_ids(id)
                .map(|p| nodes.kind(p))
                .find(|p| !matches!(p, AstKind::ParenthesizedExpression(_))),
            Some(AstKind::TSAsExpression(_) | AstKind::TSTypeAssertion(_))
        ) {
            continue;
        }
        loop {
            let (inner, ty) = match expression.without_parentheses() {
                Expression::TSAsExpression(n) => (&n.expression, &n.type_annotation),
                Expression::TSTypeAssertion(n) => (&n.expression, &n.type_annotation),
                _ => break,
            };
            only_const &= ty.is_const_type_reference();
            if !only_const {
                findings.report(node.span().start as usize, None);
                break;
            }
            expression = inner;
        }
    }
}
