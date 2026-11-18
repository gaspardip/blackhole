use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{AstKind, ast::Expression};

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
        || super::values::resolve(ctx, &p.value).is_function()
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for node in ctx.semantic.nodes().iter() {
        let AstKind::CallExpression(c) = node.kind() else {
            continue;
        };
        let inv = super::values::invoked(ctx, c);
        let Some((object, key)) = super::values::receiver(ctx, inv.callee) else {
            continue;
        };
        if super::values::any_value(ctx, object, |e| {
            matches!(
                (factory(ctx, e), key),
                (Some(Factory::Server), "listen") | (Some(Factory::Worker), "start")
            )
        }) && !strict(ctx, inv.arg(0))
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
