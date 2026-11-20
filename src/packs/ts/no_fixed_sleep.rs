use super::test_context::{runner, single_effect};
use super::values::{enclosing_call, enclosing_functions, invoked};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, NewExpression},
};
use std::collections::{HashMap, HashSet};
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
    if !super::values::any_assigned_path(ctx, timer.callee, |_, path| path == ["setTimeout"]) {
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
    let nodes = ctx.semantic.nodes();
    let mut sleeps = HashMap::new();
    for (id, n) in nodes.iter_enumerated() {
        let AstKind::AwaitExpression(a) = n.kind() else {
            continue;
        };
        let value = super::values::resolve(ctx, &a.argument);
        let sleep = if let Expression::NewExpression(n) = value {
            promise(ctx, n)
        } else {
            super::values::call_expr(value).is_some_and(|c| {
                let inv = super::values::invoked(ctx, c);
                super::values::any_assigned_path(ctx, inv.callee, |origin, path| {
                    matches!(
                        (origin, path),
                        (
                            super::values::Origin::Global,
                            ["scheduler", "wait"] | ["Bun", "sleep"]
                        ) | (
                            _,
                            ["node:timers/promises" | "timers/promises", "setTimeout"]
                        )
                    )
                })
            })
        };
        if sleep
            && let Some(function) = enclosing_functions(ctx, id).next()
            && let Some(call) = enclosing_call(ctx, function.id())
        {
            let owner = runner(ctx, &call);
            if owner.test()
                || matches!(
                    owner.first().copied(),
                    Some("beforeEach" | "beforeAll" | "afterEach" | "afterAll")
                )
            {
                sleeps.insert(id, (function.id(), a.span.start as usize));
            }
        }
    }
    if sleeps.is_empty() {
        return;
    }
    let mut active = HashSet::new();
    for (id, node) in nodes.iter_enumerated() {
        if let Some(&(function, at)) = sleeps.get(&id) {
            if !active.contains(&function) {
                findings.report(at, None);
            }
        } else if let AstKind::CallExpression(call) = node.kind()
            && let Some(function) = enclosing_functions(ctx, id).next()
        {
            match runner(ctx, &invoked(ctx, call)).short() {
                Some(["vi", "useRealTimers"]) => {
                    active.remove(&function.id());
                }
                Some(["vi", "useFakeTimers"])
                    if matches!(nodes.parent_kind(id), AstKind::ExpressionStatement(_))
                        && matches!(
                            nodes.parent_kind(nodes.parent_id(id)),
                            AstKind::FunctionBody(_)
                        ) =>
                {
                    active.insert(function.id());
                }
                _ => (),
            }
        }
    }
}
