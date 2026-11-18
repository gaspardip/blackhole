use super::test_context::{is_test, single_effect};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, NewExpression},
};
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-fixed-sleep",
    family: Family::Tests,
    provenance: "Port of landmines testing/no-fixed-sleep.",
    fix: "Await the operation or a promise resolved by its handler. Use fake timers to test elapsed time.",
    run: Run::Ts(check),
};
fn calls_resolver(body: Option<&Expression<'_>>, name: &str) -> bool {
    let Some(Expression::CallExpression(c)) = body else {
        return false;
    };
    c.callee.is_specific_id(name)
}
fn promise<'a>(ctx: &TsCtx<'a>, n: &'a NewExpression<'a>) -> bool {
    if !super::values::resolves_to_global(ctx, &n.callee, "Promise") {
        return false;
    }
    let Some(callback) = n.arguments.first().and_then(|a| a.as_expression()) else {
        return false;
    };
    let params = match callback.get_inner_expression() {
        Expression::ArrowFunctionExpression(f) => &f.params,
        Expression::FunctionExpression(f) => &f.params,
        _ => return false,
    };
    let Some(resolve) = params
        .items
        .first()
        .and_then(|p| p.pattern.get_binding_identifier())
    else {
        return false;
    };
    let Some(Expression::CallExpression(c)) =
        single_effect(callback.get_inner_expression()).map(Expression::get_inner_expression)
    else {
        return false;
    };
    if !(c.callee.is_specific_id("setTimeout")
        || c.callee.is_specific_member_access("window", "setTimeout")
        || c.callee
            .is_specific_member_access("globalThis", "setTimeout"))
    {
        return false;
    }
    match c
        .arguments
        .first()
        .and_then(|a| a.as_expression())
        .map(Expression::get_inner_expression)
    {
        Some(Expression::Identifier(id)) => id.name == resolve.name,
        Some(Expression::ArrowFunctionExpression(f)) => {
            calls_resolver(f.body.as_expression(), resolve.name.as_str())
        }
        _ => false,
    }
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    let mut sleeps = Vec::new();
    for (id, n) in nodes.iter_enumerated() {
        let AstKind::AwaitExpression(a) = n.kind() else {
            continue;
        };
        let value = super::values::resolve(ctx, &a.argument);
        let sleep = if let Expression::NewExpression(n) = value {
            promise(ctx, n)
        } else {
            super::values::call_expr(value).is_some_and(|c| {
                matches!(
                    super::test_context::runner(ctx, &super::values::invoked(ctx, c)).short(),
                    Some(
                        ["scheduler", "wait"]
                            | ["Bun", "sleep"]
                            | ["node:timers/promises" | "timers/promises", "setTimeout"]
                    )
                )
            })
        };
        if sleep
            && let Some(f) = super::values::enclosing_functions(ctx, id).next()
            && let Some(AstKind::CallExpression(c)) = super::values::outer(ctx, f.id())
            && super::test_context::runner(ctx, &super::values::invoked(ctx, c))
                .first()
                .is_some_and(|s| {
                    is_test(s)
                        || matches!(*s, "beforeEach" | "beforeAll" | "afterEach" | "afterAll")
                })
        {
            sleeps.push(a.span.start as usize);
        }
    }
    if sleeps.is_empty() {
        return;
    }
    if nodes.iter().any(|n|matches!(n.kind(),AstKind::CallExpression(c) if super::test_context::runner(ctx,&super::values::invoked(ctx,c))==["vi","useFakeTimers"])){return;}
    for at in sleeps {
        findings.report(at, None);
    }
}
