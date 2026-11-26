use super::test_context::{callback_sites, single_effect};
use super::values::{Origin, any_assigned_path, call_expr, enclosing_functions, invoked, resolve};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, NewExpression},
};
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-fixed-sleep",
    family: Family::Tests,
    gate: Gate::Blocking,
    provenance: "Port of landmines testing/no-fixed-sleep.",
    fix: "With fake timers, advance them (for example, await vi.advanceTimersByTimeAsync(ms)) instead of scheduling a timeout in the test. Otherwise, await a signal from the code under test.",
    run: Run::Ts(check),
};
fn promise<'a>(ctx: &TsCtx<'a>, n: &'a NewExpression<'a>) -> bool {
    use super::values::{path, symbol};
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
    if !any_assigned_path(ctx, timer.callee, |_, path| path == ["setTimeout"]) {
        return false;
    }
    let Some(arg) = timer.arg(0).map(|e| resolve(ctx, e)) else {
        return false;
    };
    let callee = match arg {
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
            single_effect(arg)
                .and_then(|e| call_expr(resolve(ctx, e)))
                .map(|c| invoked(ctx, c).callee)
        }
        e => Some(e),
    };
    callee
        .and_then(Expression::get_identifier_reference)
        .and_then(|id| symbol(ctx, id))
        == Some(resolver)
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    if super::test_context::test_file(ctx.path) {
        super::values::tracked_uses::report(ctx, RULE.id, findings);
    }
    for (id, node) in ctx.nodes_with_ids() {
        let AstKind::AwaitExpression(wait) = node.kind() else {
            continue;
        };
        let value = resolve(ctx, &wait.argument);
        let sleep = if let Expression::NewExpression(n) = value {
            promise(ctx, n)
        } else {
            call_expr(value).is_some_and(|call| {
                any_assigned_path(ctx, invoked(ctx, call).callee, |origin, path| {
                    matches!(
                        (origin, path),
                        (Origin::Global, ["scheduler", "wait"] | ["Bun", "sleep"])
                            | (
                                _,
                                ["node:timers/promises" | "timers/promises", "setTimeout"]
                            )
                    )
                })
            })
        };
        if sleep
            && let Some(function) = enclosing_functions(ctx, id).next()
            && callback_sites(ctx, function.id())
                .any(|site| site.test() || site.runner.hook().is_some())
        {
            findings.report(wait.span.start as usize, None);
        }
    }
}
