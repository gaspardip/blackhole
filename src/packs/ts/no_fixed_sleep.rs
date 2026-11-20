use super::test_context::{callback_sites, runner, single_effect};
use super::values::{enclosing_functions, invoked};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, NewExpression},
};
use oxc::syntax::node::NodeId;
use std::collections::{HashMap, HashSet};
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-fixed-sleep",
    family: Family::Tests,
    provenance: "Port of landmines testing/no-fixed-sleep.",
    fix: "Await the operation or a promise resolved by its handler. Install fake timers in the test or beforeEach and advance them instead of waiting.",
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

struct Sleep {
    function: NodeId,
    at: usize,
}

#[derive(Default)]
struct TimerSetup {
    installed: bool,
    restored: bool,
}

fn before_each_timers(
    ctx: &TsCtx<'_>,
    at: NodeId,
    setups: &HashMap<Option<NodeId>, TimerSetup>,
) -> bool {
    let mut pending = vec![(at, false)];
    let mut seen = HashSet::new();
    let mut reached_file = false;
    while let Some((at, mut installed)) = pending.pop() {
        let scope = enclosing_functions(ctx, at)
            .next()
            .map(oxc::semantic::AstNode::id);
        if let Some(setup) = setups.get(&scope) {
            if setup.restored {
                return false;
            }
            installed |= setup.installed;
        }
        let Some(scope) = scope else {
            if !installed {
                return false;
            }
            reached_file = true;
            continue;
        };
        if seen.insert((scope, installed)) {
            // A named suite inherits hooks from where it is registered.
            let mut registrations = callback_sites(ctx, scope)
                .filter(|site| site.runner.hook().is_none())
                .peekable();
            if registrations.peek().is_none() {
                pending.push((scope, installed));
            } else {
                pending.extend(registrations.map(|site| (site.at, installed)));
            }
        }
    }
    reached_file
}

fn report_sleeps(ctx: &TsCtx<'_>, sleeps: &HashMap<NodeId, Sleep>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    let mut active = HashMap::<_, TimerSetup>::new();
    let mut pending = Vec::new();
    for (id, node) in nodes.iter_enumerated() {
        if let Some(sleep) = sleeps.get(&id) {
            pending.push((
                sleep,
                active.get(&sleep.function).map(|setup| setup.installed),
            ));
        } else if let AstKind::CallExpression(call) = node.kind() {
            let installed = match runner(ctx, &invoked(ctx, call)).short() {
                Some(["vi" | "jest", "useRealTimers"]) => false,
                Some(["vi" | "jest", "useFakeTimers"])
                    if matches!(nodes.parent_kind(id), AstKind::ExpressionStatement(_))
                        && matches!(
                            nodes.parent_kind(nodes.parent_id(id)),
                            AstKind::FunctionBody(_)
                        ) =>
                {
                    true
                }
                _ => continue,
            };
            if let Some(function) = enclosing_functions(ctx, id).next() {
                let setup = active.entry(function.id()).or_default();
                setup.installed = installed;
                setup.restored |= !installed;
            }
        }
    }

    let mut setups = HashMap::<_, TimerSetup>::new();
    for (function, timers) in active {
        for site in
            callback_sites(ctx, function).filter(|site| site.runner.hook() == Some("beforeEach"))
        {
            let suite = enclosing_functions(ctx, site.at)
                .next()
                .map(oxc::semantic::AstNode::id);
            let setup = setups.entry(suite).or_default();
            setup.installed |= timers.installed;
            setup.restored |= timers.restored;
        }
    }
    for (sleep, local) in pending {
        let enabled = local.unwrap_or_else(|| {
            !setups.is_empty()
                && callback_sites(ctx, sleep.function)
                    .filter(|site| site.test() || site.runner.hook().is_some())
                    .all(|site| site.test() && before_each_timers(ctx, site.at, &setups))
        });
        if !enabled {
            findings.report(sleep.at, None);
        }
    }
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
            && callback_sites(ctx, function.id())
                .any(|site| site.test() || site.runner.hook().is_some())
        {
            sleeps.insert(
                id,
                Sleep {
                    function: function.id(),
                    at: a.span.start as usize,
                },
            );
        }
    }
    if !sleeps.is_empty() {
        report_sleeps(ctx, &sleeps, findings);
    }
}
