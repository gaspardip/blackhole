use super::test_context::{Runner, TestOption, enclosing_runners, test_file};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{ast::ast::Expression, span::GetSpan};
use std::collections::BTreeSet;
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-skipped-tests",
    family: Family::Tests,
    provenance: "Agents skip, focus or stub out a failing test to get a green run. Upstream accepts 255, 257, 261 and 266 differ because an unbound `context` is the runner's suite alias.",
    fix: "Make the test pass or delete it with the behaviour it covered. A skipped test proves nothing.",
    run: Run::Ts(check),
};
fn body_skip<'a>(ctx: &TsCtx<'a>, e: &'a Expression<'a>) -> Option<usize> {
    if let Some(member) = e.get_member_expr()
        && matches!(
            member.object().get_inner_expression(),
            Expression::ThisExpression(_)
        )
        && let Some((span, name)) = member.static_property_info()
        && name == "skip"
    {
        return Some(span.start as usize);
    }
    super::values::resolves_to_global(ctx, e, "pending").then_some(e.span().start as usize)
}

fn node_context_skip<'a>(ctx: &TsCtx<'a>, callee: &'a Expression<'a>) -> Option<usize> {
    use super::values::{member_span, receiver, resolve, symbol};
    let (owner, "skip" | "todo") = receiver(ctx, callee)? else {
        return None;
    };
    let id = resolve(ctx, owner).get_identifier_reference()?;
    let parameter = super::test_context::callback_parameter(ctx, symbol(ctx, id)?, |site| {
        site.test() && site.runner.is_node()
    })?;
    parameter.pattern.get_binding_identifier()?;
    Some(
        callee
            .get_member_expr()
            .map_or_else(|| callee.span(), |m| member_span(ctx, m))
            .start as usize,
    )
}
fn truthy<'a>(ctx: &TsCtx<'a>, value: &'a Expression<'a>) -> Option<bool> {
    match super::values::resolve(ctx, value) {
        Expression::BooleanLiteral(v) => Some(v.value),
        Expression::StringLiteral(v) => Some(!v.value.is_empty()),
        Expression::TemplateLiteral(v) if v.expressions.is_empty() => {
            Some(!v.quasis.first()?.value.cooked.as_ref()?.is_empty())
        }
        Expression::NumericLiteral(v) => Some(v.value != 0.0 && !v.value.is_nan()),
        Expression::BigIntLiteral(v) => Some(!v.is_zero()),
        Expression::NullLiteral(_) => Some(false),
        Expression::ArrayExpression(_)
        | Expression::ObjectExpression(_)
        | Expression::RegExpLiteral(_)
        | Expression::ClassExpression(_)
        | Expression::NewExpression(_) => Some(true),
        value if value.is_function() => Some(true),
        value
            if super::values::resolves_to_global(ctx, value, "undefined")
                || super::values::resolves_to_global(ctx, value, "NaN") =>
        {
            Some(false)
        }
        _ => None,
    }
}
pub(super) fn skip_option<'a>(
    ctx: &TsCtx<'a>,
    call: &super::values::Invocation<'a>,
    runner: Runner<'a>,
) -> Option<(usize, bool)> {
    for slot in runner.test_options(ctx, call).into_iter().flatten() {
        let (arg, options) = match slot {
            TestOption::Object(arg, options) => (arg, options),
            TestOption::Opaque { at, .. } => return Some((at, false)),
            TestOption::Timeout(_) => continue,
        };
        for key in ["todo", "skip"] {
            if let Some(property) = options.named.get(key) {
                let enabled = truthy(ctx, &property.value);
                if enabled != Some(false) {
                    return Some((
                        property.key.span().start as usize,
                        key == "todo" && enabled == Some(true),
                    ));
                }
            }
        }
        if options.unknown {
            return Some((arg.span().start as usize, false));
        }
    }
    None
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    super::values::tracked_uses::report(ctx, RULE.id, findings);
    let mut locations = BTreeSet::new();
    let nodes = ctx.semantic.nodes();
    let is_test_file = test_file(ctx.path);
    for (id, n) in nodes.iter_enumerated() {
        let Some(c) = super::values::site(ctx, n.kind()) else {
            continue;
        };
        let call = c.call;
        let path = super::test_context::runner(ctx, &call);
        let position = if path.test_or_suite() {
            if path.skipped {
                let usage = c.callee.get_inner_expression();
                Some(if matches!(usage, Expression::Identifier(_)) {
                    usage.span().start as usize
                } else {
                    path.skip_at.unwrap_or(call.callee.span().start as usize)
                })
            } else {
                skip_option(ctx, &call, path).map(|(at, _)| at)
            }
        } else if path.legacy_skip()
            || path.is_empty()
                && super::values::any_assigned_path(ctx, call.callee, |origin, parts| {
                    Runner::from_path(origin, parts).is_some_and(|r| r.test_or_suite() && r.skipped)
                })
        {
            Some(call.callee.span().start as usize)
        } else if let Some(at) = node_context_skip(ctx, call.callee) {
            Some(at)
        } else if is_test_file
            && let Some(at) = body_skip(ctx, call.callee)
            && enclosing_runners(ctx, id).any(|r| r.test())
        {
            Some(at)
        } else {
            None
        };
        locations.extend(position);
    }
    for at in locations {
        findings.report(at, None);
    }
}
