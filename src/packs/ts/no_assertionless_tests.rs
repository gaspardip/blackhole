use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{AstKind, ast::Expression},
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
fn same<'a>(ctx: &TsCtx<'a>, a: &'a Expression<'a>, b: &'a Expression<'a>) -> bool {
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
fn tautology<'a>(
    ctx: &TsCtx<'a>,
    call: &super::values::Invocation<'a>,
    parts: super::test_context::Runner<'a>,
) -> bool {
    use super::values::{call_expr, invoked, receiver, resolve};
    let first = call.arg(0);
    if parts == ["assert"] {
        return first.is_some_and(literal);
    }
    if parts.first() == Some(&"assert") {
        return first.zip(call.arg(1)).is_some_and(|(a, b)| same(ctx, a, b));
    }
    let Some((object, _)) = receiver(ctx, call.callee) else {
        return false;
    };
    let mut object = resolve(ctx, object);
    while let Some(m) = object.get_member_expr() {
        object = m.object().get_inner_expression();
    }
    let Some(expect) = call_expr(object) else {
        return false;
    };
    let expect = invoked(ctx, expect);
    super::test_context::runner(ctx, &expect) == ["expect"]
        && expect.arg(0).is_some_and(|subject| {
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
        let inv = super::values::invoked(ctx, c);
        let parts = super::test_context::runner(ctx, &inv);
        if parts.is_empty()
            && let Some(id) = c.callee.get_identifier_reference()
        {
            let name = id.name.as_str();
            if (name.starts_with("expect") && name != "expect")
                || (name.starts_with("assert") && name != "assert")
            {
                assertions.entry(c.span.start).or_default();
            }
        }
        let Some(&base) = parts.first() else {
            continue;
        };
        if base.starts_with("expect") || base.starts_with("assert") {
            if matches!(
                parts.short(),
                Some([
                    "expect",
                    "assertions"
                        | "hasAssertions"
                        | "any"
                        | "anything"
                        | "arrayContaining"
                        | "arrayOf"
                        | "objectContaining"
                        | "stringContaining"
                        | "stringMatching"
                        | "closeTo"
                ])
            ) {
                continue;
            }
            let bare_literal = parts == ["expect"]
                && inv.arg(0).is_some_and(literal)
                && super::values::outer(ctx, n.id())
                    .and_then(|kind| kind.as_member_expression_kind())
                    .is_none();
            let found = bare_literal || tautology(ctx, &inv, parts);
            *assertions.entry(c.span.start).or_default() |= found;
        }
        if !parts.test()
            || parts.todo
            || c.callee.get_member_expr().is_some_and(|m| {
                matches!(
                    super::values::MemberKeys(ctx).read(m),
                    Some("each" | "for" | "extend")
                )
            })
        {
            continue;
        }
        // shortcut: callbacks passed by reference need interprocedural analysis before their assertions can be judged.
        if let Some(body) = inv.inline_test_body() {
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
