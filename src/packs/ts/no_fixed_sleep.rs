use super::test_context::{callback_sites, registration_sites, runner, single_effect};
use super::values::{Invocation, enclosing_functions, invoked};
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
    fix: "Await the operation or a promise resolved by its handler. Install fake timers without timer exclusions in the test or beforeEach and advance them instead of waiting.",
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
    blocked: bool,
}

fn fakes_timeouts(invocation: &Invocation<'_>) -> bool {
    if invocation.spread_args {
        return false;
    }
    match invocation.args.as_slice() {
        [] => true,
        [Some(options)] => matches!(
            options.get_inner_expression(),
            Expression::ObjectExpression(object) if object.properties.iter().all(|field| {
                field.as_property().and_then(|p| p.key.static_name())
                    .is_some_and(|key| key != "doNotFake" && key != "toFake")
            })
        ),
        _ => false,
    }
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
            if setup.blocked {
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
            let Some(registrations) = registration_sites(ctx, scope) else {
                return false;
            };
            pending.extend(registrations.into_iter().map(|site| (site, installed)));
        }
    }
    reached_file
}

fn report_sleeps(ctx: &TsCtx<'_>, sleeps: &HashMap<NodeId, Vec<Sleep>>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    let mut active = HashMap::<_, TimerSetup>::new();
    let mut pending = Vec::new();
    for (id, node) in nodes.iter_enumerated() {
        if let Some(sleeps) = sleeps.get(&id) {
            let enclosing = enclosing_functions(ctx, id)
                .next()
                .map(oxc::semantic::AstNode::id);
            for sleep in sleeps {
                let local = if enclosing == Some(sleep.function) {
                    active.get(&sleep.function).map(|setup| setup.installed)
                } else {
                    Some(false)
                };
                pending.push((sleep, local));
            }
        } else if let AstKind::CallExpression(call) = node.kind() {
            let invocation = invoked(ctx, call);
            let installed = match runner(ctx, &invocation).short() {
                Some(["vi" | "jest", "useRealTimers"]) => false,
                Some(["vi" | "jest", "useFakeTimers"]) if !fakes_timeouts(&invocation) => false,
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
                setup.blocked |= !installed;
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
            setup.blocked |= timers.blocked;
        }
    }
    for (sleep, local) in pending {
        let enabled = local.unwrap_or_else(|| {
            !setups.is_empty()
                && registration_sites(ctx, sleep.function).is_some_and(|sites| {
                    sites
                        .into_iter()
                        .all(|site| before_each_timers(ctx, site, &setups))
                })
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
    let mut sleeps = HashMap::<_, Vec<_>>::new();
    for (id, n) in nodes.iter_enumerated() {
        let AstKind::AwaitExpression(a) = n.kind() else {
            continue;
        };
        let value = super::values::resolve(ctx, &a.argument);
        let scheduled = if let Expression::NewExpression(n) = value {
            // The Promise constructor runs its timer executor synchronously.
            promise(ctx, n).then_some(n.node_id())
        } else {
            super::values::call_expr(value)
                .filter(|c| {
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
                .map(oxc::ast::ast::CallExpression::node_id)
        };
        if let Some(scheduled) = scheduled
            && let Some(function) = enclosing_functions(ctx, id).next()
            && callback_sites(ctx, function.id())
                .any(|site| site.test() || site.runner.hook().is_some())
        {
            sleeps.entry(scheduled).or_default().push(Sleep {
                function: function.id(),
                at: a.span.start as usize,
            });
        }
    }
    if !sleeps.is_empty() {
        report_sleeps(ctx, &sleeps, findings);
    }
}
