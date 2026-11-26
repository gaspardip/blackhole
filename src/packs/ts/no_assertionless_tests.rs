use super::test_context::{callback_body, callback_kind};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::{ast::ast::Expression, span::GetSpan};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-assertionless-tests",
    family: Family::Tests,
    gate: Gate::Blocking,
    provenance: "Agents write tests that exercise code without assertions or assert a constant.",
    fix: "Assert the behaviour the test names, on a value the code under test produced.",
    run: Run::Ts(check),
};
fn literal(ctx: &TsCtx<'_>, e: &Expression<'_>) -> bool {
    let e = super::values::unwrapped(ctx, e);
    e.is_literal() || matches!(e,Expression::TemplateLiteral(t) if t.expressions.is_empty())
}
fn same<'a>(ctx: &TsCtx<'a>, a: &'a Expression<'a>, b: &'a Expression<'a>) -> bool {
    use super::values::{resolve, same_key, same_string_bytes, symbol, unwrapped};
    let (a, b) = (unwrapped(ctx, a), unwrapped(ctx, b));
    match (a, b) {
        (Expression::Identifier(_), Expression::Identifier(_)) => {
            let (a, b) = (resolve(ctx, a), resolve(ctx, b));
            match (a, b) {
                (Expression::Identifier(x), Expression::Identifier(y)) => {
                    match (symbol(ctx, x), symbol(ctx, y)) {
                        (Some(x), Some(y)) => x == y,
                        (None, None) => same_key(ctx, x.name.as_str(), y.name.as_str()),
                        _ => false,
                    }
                }
                _ => a.span() == b.span(),
            }
        }
        (Expression::StringLiteral(x), Expression::StringLiteral(y)) => {
            same_string_bytes(ctx, x.value.as_bytes(), y.value.as_bytes())
        }
        (Expression::NumericLiteral(x), Expression::NumericLiteral(y)) => {
            x.value.to_bits() == y.value.to_bits()
        }
        (Expression::BigIntLiteral(x), Expression::BigIntLiteral(y)) => {
            same_key(ctx, x.value.as_str(), y.value.as_str())
        }
        (x, y) if literal(ctx, x) && literal(ctx, y) => same_key(
            ctx,
            x.span().source_text(ctx.text),
            y.span().source_text(ctx.text),
        ),
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
        return first.is_some_and(|arg| literal(ctx, arg));
    }
    if parts.first() == Some(&"assert") {
        return first.zip(call.arg(1)).is_some_and(|(a, b)| same(ctx, a, b));
    }
    let Some((object, _)) = receiver(ctx, call.callee) else {
        return false;
    };
    let mut object = resolve(ctx, object);
    while let Some(m) = object.get_member_expr() {
        ctx.record_work(1);
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
        literal(ctx, resolve(ctx, subject))
            && call.unreadable_args.is_none()
            && ctx
                .counted(call.args.iter())
                .all(|arg| arg.is_some_and(|value| literal(ctx, resolve(ctx, value))))
            || first.is_some_and(|arg| same(ctx, subject, arg))
    })
}
pub(super) struct Inventory {
    cases: usize,
    assertions: BTreeMap<u32, Option<usize>>,
    helpers: BTreeMap<u32, oxc::span::Span>,
    tests: Vec<(usize, oxc::span::Span)>,
}

impl Inventory {
    pub(super) fn counts(&self) -> (usize, usize) {
        (self.cases, self.assertions.len())
    }
}

pub(super) fn inventory(ctx: &TsCtx<'_>) -> Inventory {
    let mut cases = 0;
    let mut assertions = BTreeMap::<u32, Option<usize>>::new();
    let mut helpers = BTreeMap::new();
    let mut tests = Vec::new();
    for n in ctx.nodes() {
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
            && let Some(body) = callback_kind(ctx, inv.callee).and_then(callback_body)
        {
            helpers.insert(c.span.start, body);
        }
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
                        .is_some_and(|value| literal(ctx, super::values::resolve(ctx, value)))
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
        cases += 1;
        if let Some(body) = inv.test_body(ctx) {
            tests.push((position(), body));
        }
    }
    Inventory {
        cases,
        assertions,
        helpers,
        tests,
    }
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let Inventory {
        tests,
        assertions,
        helpers,
        ..
    } = inventory(ctx);
    report_assertions(ctx, tests, &assertions, &helpers, findings);
}

fn report_assertions(
    ctx: &TsCtx<'_>,
    tests: Vec<(usize, oxc::span::Span)>,
    assertions: &BTreeMap<u32, Option<usize>>,
    helpers: &BTreeMap<u32, oxc::span::Span>,
    findings: &mut Findings,
) {
    let mut at = BTreeSet::new();
    for (start, body) in tests {
        let mut pending = vec![body];
        let mut seen = std::collections::HashSet::new();
        let mut asserted = false;
        while let Some(body) = pending.pop() {
            ctx.record_work(1);
            if !seen.insert(body) {
                continue;
            }
            let mut scope = assertions.range(body.start..body.end).peekable();
            asserted |= scope.peek().is_some();
            at.extend(scope.filter_map(|(_, &finding)| {
                ctx.record_work(1);
                finding
            }));
            pending.extend(helpers.range(body.start..body.end).map(|(_, &helper)| {
                ctx.record_work(1);
                helper
            }));
        }
        if !asserted {
            at.insert(start);
        }
    }
    for pos in at {
        findings.report(pos, None);
    }
}
