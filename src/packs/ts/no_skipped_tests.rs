use super::test_context::{Runner, enclosing_runners, test_file};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, TSType},
    },
    span::GetSpan,
};
use std::collections::BTreeSet;
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-skipped-tests",
    family: Family::Tests,
    provenance: "Agents skip, focus or stub out a failing test to get a green run. Upstream accepts 255, 257, 261 and 266 differ because an unbound `context` is the runner's suite alias.",
    fix: "Make the test pass or delete it with the behaviour it covered. A skipped test proves nothing.",
    run: Run::Ts(check),
};
pub(super) fn modifier(name: &str) -> bool {
    matches!(
        name,
        "skip" | "only" | "todo" | "skipIf" | "runIf" | "fixme" | "fails" | "failing"
    )
}
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
fn readable_non_options<'a>(ctx: &TsCtx<'a>, arg: &'a Expression<'a>) -> bool {
    let value = super::values::resolve(ctx, arg);
    if value.is_function()
        || value.is_literal()
        || super::values::resolves_to_global(ctx, value, "undefined")
    {
        return true;
    }
    [value, arg].into_iter().any(|expr| {
        let Some(declaration) = expr
            .get_identifier_reference()
            .and_then(|id| super::values::declaration(ctx, id))
        else {
            return false;
        };
        let annotation = match declaration {
            AstKind::Function(_) => return true,
            AstKind::FormalParameter(p) => p.type_annotation.as_deref(),
            AstKind::VariableDeclarator(v) => v.type_annotation.as_deref(),
            _ => None,
        };
        annotation.is_some_and(|a| matches!(a.type_annotation, TSType::TSFunctionType(_)))
    })
}

pub(super) fn skip_option<'a>(
    ctx: &TsCtx<'a>,
    call: &super::values::Invocation<'a>,
    runner: Runner<'a>,
) -> Option<(usize, bool)> {
    for index in 0..2 {
        let Some(arg) = call.arg(index) else {
            continue;
        };
        let Some(options) = super::values::properties(ctx, arg) else {
            if index == 1 && runner.accepts_options_second() && !readable_non_options(ctx, arg) {
                return Some((arg.span().start as usize, false));
            }
            continue;
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
        let base = path.first().copied().unwrap_or("");
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
        } else if matches!(base, "xit" | "xtest" | "xdescribe" | "fit" | "fdescribe")
            || path.is_empty()
                && super::values::any_assigned_path(ctx, call.callee, |origin, parts| {
                    let ((super::values::Origin::Global, names)
                    | (
                        _,
                        [
                            "vitest" | "node:test" | "@jest/globals" | "@playwright/test",
                            names @ ..,
                        ],
                    )) = (origin, parts)
                    else {
                        return false;
                    };
                    names.split_first().is_some_and(|(root, rest)| {
                        (super::test_context::is_test(root) || super::test_context::suite(root))
                            && rest.iter().any(|key| modifier(key))
                    })
                })
        {
            Some(call.callee.span().start as usize)
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
