use super::test_context::{enclosing_runners, single_effect};
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
fn promise<'a>(ctx: &TsCtx<'a>, n: &'a NewExpression<'a>) -> bool {
    use super::values::{call_expr, invoked, path, resolve, symbol};
    if path(ctx, &n.callee).as_deref() != Some(&["Promise"]) {
        return false;
    }
    let Some(callback) = n
        .arguments
        .first()
        .and_then(|a| a.as_expression())
        .map(|e| resolve(ctx, e))
    else {
        return false;
    };
    let params = match callback {
        Expression::ArrowFunctionExpression(f) => &f.params,
        Expression::FunctionExpression(f) => &f.params,
        _ => return false,
    };
    let Some(resolver) = params
        .items
        .first()
        .and_then(|p| p.pattern.get_binding_identifier())
        .and_then(|id| id.symbol_id.get())
    else {
        return false;
    };
    let Some(effect) = single_effect(callback).and_then(|e| call_expr(resolve(ctx, e))) else {
        return false;
    };
    let timer = invoked(ctx, effect);
    if timer.path.as_deref() != Some(&["setTimeout"]) {
        return false;
    }
    let Some(arg) = timer.arg(0).map(|e| resolve(ctx, e)) else {
        return false;
    };
    let callee = match arg {
        Expression::ArrowFunctionExpression(f) => f
            .body
            .as_expression()
            .and_then(call_expr)
            .map(|c| invoked(ctx, c).callee),
        e => Some(e),
    };
    callee
        .and_then(Expression::get_identifier_reference)
        .and_then(|id| symbol(ctx, id))
        == Some(resolver)
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
            && enclosing_runners(ctx, id).next().is_some_and(|owner| {
                owner.test()
                    || matches!(
                        owner.first().copied(),
                        Some("beforeEach" | "beforeAll" | "afterEach" | "afterAll")
                    )
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
