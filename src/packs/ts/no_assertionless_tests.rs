use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{ast::ast::Expression, span::GetSpan};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-assertionless-tests",
    family: Family::Tests,
    provenance: "Agents write tests that exercise code without assertions or assert a constant.",
    fix: "Assert the behaviour the test names, on a value the code under test produced.",
    run: Run::Ts(check),
};
fn literal(e: &Expression<'_>) -> bool {
    let e = super::values::unwrapped(e);
    e.is_literal() || matches!(e,Expression::TemplateLiteral(t) if t.expressions.is_empty())
}
fn same<'a>(ctx: &TsCtx<'a>, a: &'a Expression<'a>, b: &'a Expression<'a>) -> bool {
    use super::values::{resolve, symbol, unwrapped};
    let (a, b) = (unwrapped(a), unwrapped(b));
    match (a, b) {
        (Expression::Identifier(_), Expression::Identifier(_)) => {
            let (a, b) = (resolve(ctx, a), resolve(ctx, b));
            match (a, b) {
                (Expression::Identifier(x), Expression::Identifier(y)) => {
                    match (symbol(ctx, x), symbol(ctx, y)) {
                        (Some(x), Some(y)) => x == y,
                        (None, None) => x.name == y.name,
                        _ => false,
                    }
                }
                _ => a.span() == b.span(),
            }
        }
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
    matches!(
        super::test_context::runner(ctx, &expect).short(),
        Some(["expect"] | ["expect", "soft"])
    ) && expect.arg(0).is_some_and(|subject| {
        literal(resolve(ctx, subject)) || first.is_some_and(|arg| same(ctx, subject, arg))
    })
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut assertions = BTreeMap::<u32, Option<usize>>::new();
    let mut tests = Vec::new();
    for n in ctx.semantic.nodes().iter() {
        let Some(c) = super::values::site(ctx, n.kind()) else {
            continue;
        };
        let inv = c.call;
        let position = || match n.kind() {
            oxc::ast::AstKind::CallExpression(call) => super::values::call_position(ctx, call),
            _ => c.span.start as usize,
        };
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
            if matches!(
                parts.short(),
                Some(["expect"] | ["expect", "soft" | "poll"])
            ) {
                if parts == ["expect"]
                    && inv
                        .arg(0)
                        .is_some_and(|value| literal(super::values::resolve(ctx, value)))
                    && super::values::outer(ctx, n.id())
                        .and_then(|kind| kind.as_member_expression_kind())
                        .is_none()
                {
                    assertions.insert(c.span.start, Some(position()));
                }
                continue;
            }
            let finding = assertions.entry(c.span.start).or_default();
            if tautology(ctx, &inv, parts) {
                *finding = Some(position());
            }
        }
        if !parts.test()
            || parts.todo
            || super::no_skipped_tests::skip_option(ctx, &inv, parts).is_some_and(|(_, todo)| todo)
            || c.callee.get_member_expr().is_some_and(|m| {
                matches!(
                    super::values::MemberKeys(ctx).read(m),
                    Some("each" | "for" | "extend")
                )
            })
        {
            continue;
        }
        if let Some(body) = inv.test_body(ctx) {
            tests.push((position(), body));
        }
    }
    let mut at = BTreeSet::new();
    for (start, body) in tests {
        let mut scope = assertions.range(body.start..body.end).peekable();
        if scope.peek().is_none() {
            at.insert(start);
        } else {
            at.extend(scope.filter_map(|(_, &finding)| finding));
        }
    }
    for pos in at {
        findings.report(pos, None);
    }
}
