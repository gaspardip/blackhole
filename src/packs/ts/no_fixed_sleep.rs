use super::test_context::{runner, single_effect};
use super::values::{enclosing_call, enclosing_functions, invoked};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, NewExpression},
};
use oxc::syntax::node::NodeId;
use std::collections::{BTreeMap, HashMap};
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
    test: bool,
}

#[derive(Default)]
struct Setup {
    all: Option<bool>,
    each: Option<bool>,
}

fn report_sleeps(ctx: &TsCtx<'_>, sleeps: &HashMap<NodeId, Sleep>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    let mut active = BTreeMap::new();
    let mut pending = Vec::new();
    for (id, node) in nodes.iter_enumerated() {
        if let Some(sleep) = sleeps.get(&id) {
            pending.push((sleep, active.get(&Some(sleep.function)).copied()));
        } else if let AstKind::CallExpression(call) = node.kind() {
            let runner = runner(ctx, &invoked(ctx, call));
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
        let Some(call) = enclosing_call(ctx, function) else {
            continue;
        };
        let suite = enclosing_functions(ctx, function)
            .next()
            .map(oxc::semantic::AstNode::id);
        match runner(ctx, &call).hook() {
            Some("beforeAll") => setups.entry(suite).or_default().all = Some(enabled),
            Some("beforeEach") => setups.entry(suite).or_default().each = Some(enabled),
            _ => (),
        }
    }
    for (sleep, local) in pending {
        let enabled = local
            .or_else(|| {
                if !sleep.test {
                    return None;
                }
                let scopes = enclosing_functions(ctx, sleep.function)
                    .map(|node| Some(node.id()))
                    .chain(std::iter::once(None));
                let mut before_all = None;
                for setup in scopes.filter_map(|scope| setups.get(&scope)) {
                    if setup.each.is_some() {
                        return setup.each;
                    }
                    before_all = before_all.or(setup.all);
                }
                before_all
            })
            .or_else(|| active.get(&None).copied());
        if enabled != Some(true) {
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
            && let Some(call) = enclosing_call(ctx, function.id())
        {
            let owner = runner(ctx, &call);
            let test = owner.test() && owner.hook().is_none();
            if test || owner.hook().is_some() {
                sleeps.insert(
                    id,
                    Sleep {
                        function: function.id(),
                        at: a.span.start as usize,
                        test,
                    },
                );
            }
        }
    }
    if !sleeps.is_empty() {
        report_sleeps(ctx, &sleeps, findings);
    }
}
