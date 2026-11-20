use super::test_context::{CallbackSite, callback_sites, runner, single_effect};
use super::values::{enclosing_functions, invoked};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, NewExpression},
};
use oxc::{
    span::{GetSpan, Span},
    syntax::node::NodeId,
};
use std::collections::{BTreeSet, HashMap};
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

struct Sleep {
    function: NodeId,
    at: usize,
}

#[derive(Default)]
struct Setup {
    all: Option<(NodeId, bool)>,
    each: Option<(NodeId, bool)>,
    restores_after_each: bool,
}

fn enclosing_suites<'ctx>(
    ctx: &'ctx TsCtx<'_>,
    at: NodeId,
) -> impl Iterator<Item = Option<NodeId>> + 'ctx {
    enclosing_functions(ctx, at)
        .map(|node| Some(node.id()))
        .chain(std::iter::once(None))
}

#[derive(Default)]
struct TestOrder {
    last: HashMap<Option<NodeId>, u32>,
    restores: BTreeSet<u32>,
}

impl TestOrder {
    fn record(
        &mut self,
        ctx: &TsCtx<'_>,
        setups: &HashMap<Option<NodeId>, Setup>,
        at: NodeId,
        start: u32,
    ) {
        for suite in enclosing_suites(ctx, at) {
            let last = self.last.entry(suite).or_default();
            *last = (*last).max(start);
            if setups
                .get(&suite)
                .is_some_and(|setup| setup.restores_after_each)
            {
                self.restores.insert(start);
            }
        }
    }
}

fn setup_timers(
    ctx: &TsCtx<'_>,
    callback: CallbackSite<'_>,
    setups: &HashMap<Option<NodeId>, Setup>,
    order: &TestOrder,
    file_timers: bool,
) -> bool {
    let mut suites = enclosing_suites(ctx, callback.at);
    let suite = suites.next().flatten();
    let extent = |scope: Option<NodeId>| {
        scope.map_or(Span::new(0, u32::MAX), |id| {
            ctx.semantic.nodes().get_node(id).span()
        })
    };
    let point = match callback.runner.hook() {
        Some("beforeAll" | "before") => extent(suite).start,
        Some("afterAll" | "after") => extent(suite).end,
        // The last run can observe every restore that precedes a per-test hook.
        Some("beforeEach" | "afterEach") => {
            order.last.get(&suite).copied().unwrap_or(callback.start)
        }
        _ => callback.start,
    };
    let mut before_all = None;
    for scope in std::iter::once(suite).chain(suites) {
        let Some(setup) = setups.get(&scope) else {
            continue;
        };
        if callback.test()
            && let Some((_, enabled)) = setup.each
        {
            return enabled;
        }
        if before_all.is_none()
            && let Some((_, enabled)) = setup.all
        {
            let start = extent(scope).start;
            if start < point {
                before_all = Some((start, enabled));
            }
        }
    }
    let (installed, enabled) = before_all.unwrap_or((0, file_timers));
    enabled && order.restores.range(installed..point).next().is_none()
}

fn report_sleeps(ctx: &TsCtx<'_>, sleeps: &HashMap<NodeId, Sleep>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    let mut active = HashMap::new();
    let mut pending = Vec::new();
    let mut tests = Vec::new();
    for (id, node) in nodes.iter_enumerated() {
        if let Some(sleep) = sleeps.get(&id) {
            pending.push((sleep, active.get(&Some(sleep.function)).copied()));
        } else if let AstKind::CallExpression(call) = node.kind() {
            let invocation = invoked(ctx, call);
            let runner = runner(ctx, &invocation);
            if runner.test()
                && runner.hook().is_none()
                && !matches!(runner.last, Some("skip" | "todo" | "fixme"))
                && (runner.first().copied() == runner.last || invocation.test_body(ctx).is_some())
            {
                tests.push((id, invocation.span.start));
            }
            let next = match runner.short() {
                Some(["vi", "useRealTimers"]) => false,
                Some(["vi", "useFakeTimers"])
                    if matches!(nodes.parent_kind(id), AstKind::ExpressionStatement(_))
                        && matches!(
                            nodes.parent_kind(nodes.parent_id(id)),
                            AstKind::FunctionBody(_) | AstKind::Program(_)
                        ) =>
                {
                    true
                }
                _ => continue,
            };
            active.insert(
                enclosing_functions(ctx, id)
                    .next()
                    .map(oxc::semantic::AstNode::id),
                next,
            );
        }
    }

    let mut setups = HashMap::<_, Setup>::new();
    for (&function, &enabled) in &active {
        let Some(function) = function else { continue };
        for site in callback_sites(ctx, function) {
            let suite = enclosing_functions(ctx, site.at)
                .next()
                .map(oxc::semantic::AstNode::id);
            let setup = match site.runner.hook() {
                Some("beforeAll") => &mut setups.entry(suite).or_default().all,
                Some("beforeEach") => &mut setups.entry(suite).or_default().each,
                Some("afterEach") if !enabled => {
                    setups.entry(suite).or_default().restores_after_each = true;
                    continue;
                }
                _ => continue,
            };
            if setup.is_none_or(|(at, _)| at < site.at) {
                *setup = Some((site.at, enabled));
            }
        }
    }
    let mut order = TestOrder::default();
    for (at, start) in tests {
        order.record(ctx, &setups, at, start);
    }
    let file_timers = active.get(&None) == Some(&true);
    for (sleep, local) in pending {
        let enabled = local.unwrap_or_else(|| {
            callback_sites(ctx, sleep.function)
                .filter(|site| site.test() || site.runner.hook().is_some())
                .all(|site| setup_timers(ctx, site, &setups, &order, file_timers))
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
