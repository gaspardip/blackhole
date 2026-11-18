use super::values::symbol;
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{AssignmentTarget, Expression},
};
use oxc::syntax::symbol::SymbolId;
use std::collections::HashMap;

pub(crate) const RULE: Rule = Rule {
    id: "ts/strict-msw",
    family: Family::Tests,
    provenance: "Port of landmines testing/strict-msw.",
    fix: "Start MSW with onUnhandledRequest set to error. Add handlers for the requests the test makes.",
    run: Run::Ts(check),
};
#[derive(Clone, Copy, PartialEq)]
enum Factory {
    Server,
    Worker,
    Ambiguous,
}
fn factory<'a>(ctx: &TsCtx<'a>, e: &'a Expression<'a>) -> Option<Factory> {
    let call = super::values::call_expr(super::values::resolve(ctx, e))?;
    let inv = super::values::invoked(ctx, call);
    let [source, name] = inv.path.as_deref()? else {
        return None;
    };
    if *source != "msw" && !source.starts_with("msw/") {
        return None;
    }
    match *name {
        "setupServer" => Some(Factory::Server),
        "setupWorker" => Some(Factory::Worker),
        _ => None,
    }
}
fn strict<'a>(ctx: &TsCtx<'a>, opts: Option<&'a Expression<'a>>) -> bool {
    let Some(p) = opts
        .and_then(|e| super::values::properties(ctx, e))
        .and_then(|p| p.named.get("onUnhandledRequest").copied())
    else {
        return false;
    };
    super::values::literal(ctx, &p.value) == Some("error")
        || matches!(
            super::values::resolve(ctx, &p.value),
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
        )
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut factories = HashMap::<SymbolId, Factory>::new();
    for node in ctx.semantic.nodes().iter() {
        let (symbol, value) = match node.kind() {
            AstKind::VariableDeclarator(v) => (
                v.id.get_binding_identifier()
                    .and_then(|b| b.symbol_id.get()),
                v.init.as_ref(),
            ),
            AstKind::AssignmentExpression(a) => {
                if let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                    (symbol(ctx, id), Some(&a.right))
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        if let (Some(symbol), Some(kind)) = (symbol, value.and_then(|v| factory(ctx, v))) {
            factories
                .entry(symbol)
                .and_modify(|previous| {
                    if *previous != kind {
                        *previous = Factory::Ambiguous;
                    }
                })
                .or_insert(kind);
        }
    }
    for node in ctx.semantic.nodes().iter() {
        let AstKind::CallExpression(c) = node.kind() else {
            continue;
        };
        let inv = super::values::invoked(ctx, c);
        let Some((object, key)) = super::values::receiver(ctx, inv.callee) else {
            continue;
        };
        let kind = match super::values::resolve(ctx, object) {
            Expression::Identifier(id) => symbol(ctx, id).and_then(|s| factories.get(&s).copied()),
            other => factory(ctx, other),
        };
        if matches!(
            (kind, Some(key)),
            (Some(Factory::Server), Some("listen")) | (Some(Factory::Worker), Some("start"))
        ) && !strict(ctx, inv.arg(0))
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
