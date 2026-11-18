use super::test_context::{chain_property, is_test, root};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{CallExpression, Expression},
    },
    span::GetSpan,
};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-assertionless-tests",
    family: Family::Tests,
    provenance: "Agents write tests that exercise code without assertions or assert a constant.",
    fix: "Assert the behaviour the test names, on a value the code under test produced.",
    run: Run::Ts(check),
};
fn literal(e: &Expression<'_>) -> bool {
    let e = e.get_inner_expression();
    e.is_literal() || matches!(e,Expression::TemplateLiteral(t) if t.expressions.is_empty())
}
fn same(ctx: &TsCtx<'_>, a: &Expression<'_>, b: &Expression<'_>) -> bool {
    match (a.get_inner_expression(), b.get_inner_expression()) {
        (Expression::Identifier(x), Expression::Identifier(y)) => x.name == y.name,
        (Expression::StringLiteral(x), Expression::StringLiteral(y)) => x.value == y.value,
        (Expression::NumericLiteral(x), Expression::NumericLiteral(y)) => {
            x.value.to_bits() == y.value.to_bits()
        }
        (Expression::BigIntLiteral(x), Expression::BigIntLiteral(y)) => x.value == y.value,
        (x, y) if literal(x) && literal(y) => {
            x.span().source_text(ctx.text) == y.span().source_text(ctx.text)
        }
        _ => false,
    }
}
fn tautology(ctx: &TsCtx<'_>, call: &CallExpression<'_>) -> bool {
    let first = call.arguments.first().and_then(|a| a.as_expression());
    if matches!(&call.callee,Expression::Identifier(id) if id.name=="assert") {
        return first.is_some_and(literal);
    }
    let Some(m) = call.callee.get_member_expr() else {
        return false;
    };
    if root(&call.callee) == Some("assert") {
        return first
            .zip(call.arguments.get(1).and_then(|a| a.as_expression()))
            .is_some_and(|(a, b)| same(ctx, a, b));
    }
    let mut object = m.object().get_inner_expression();
    while let Some(n) = object.get_member_expr() {
        object = n.object().get_inner_expression();
    }
    let Expression::CallExpression(expect) = object else {
        return false;
    };
    matches!(&expect.callee,Expression::Identifier(id) if id.name=="expect")
        && expect
            .arguments
            .first()
            .and_then(|a| a.as_expression())
            .is_some_and(|subject| {
                literal(subject) || first.is_some_and(|arg| same(ctx, subject, arg))
            })
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut assertions = BTreeMap::<u32, bool>::new();
    let mut tests = Vec::new();
    for n in ctx.semantic.nodes().iter() {
        let AstKind::CallExpression(c) = n.kind() else {
            continue;
        };
        let Some(base) = root(&c.callee) else {
            continue;
        };
        if base.starts_with("expect") || base.starts_with("assert") {
            let found = tautology(ctx, c);
            *assertions.entry(c.span.start).or_default() |= found;
        }
        if !is_test(base)
            || chain_property(&c.callee, |s| s == "todo").is_some()
            || super::test_context::static_method(&c.callee, &["each", "for", "extend"])
        {
            continue;
        }
        // shortcut: callbacks passed by reference need interprocedural analysis before their assertions can be judged.
        if let Some(body) = c.arguments.iter().skip(1).find_map(|a| {
            match a.as_expression()?.get_inner_expression() {
                Expression::ArrowFunctionExpression(f) => Some(f.body.span()),
                Expression::FunctionExpression(f) => Some(f.body.as_ref()?.span),
                _ => None,
            }
        }) {
            tests.push((c.span.start, body));
        }
    }
    let mut at = BTreeSet::new();
    for (start, body) in tests {
        let mut scope = assertions.range(body.start..body.end).peekable();
        if scope.peek().is_none() {
            at.insert(start);
        } else {
            at.extend(scope.filter_map(|(&pos, &bad)| bad.then_some(pos)));
        }
    }
    for pos in at {
        findings.report(pos as usize, None);
    }
}
