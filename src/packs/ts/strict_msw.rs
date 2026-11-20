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
    let value = super::values::resolve(ctx, e);
    let inv = if let Some(c) = super::values::call_expr(value) {
        super::values::invoked(ctx, c)
    } else {
        super::values::site(
            ctx,
            match value {
                Expression::TaggedTemplateExpression(t) => AstKind::TaggedTemplateExpression(t),
                Expression::NewExpression(n) => AstKind::NewExpression(n),
                _ => return None,
            },
        )?
        .call
    };
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
pub(super) fn tracked<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> bool {
    let value = super::values::resolve(ctx, expr);
    if factory(ctx, value).is_some() {
        return true;
    }
    super::values::receiver(ctx, value).is_some_and(|(owner, key)| {
        matches!(
            (factory(ctx, owner), key),
            (Some(Factory::Server), "listen") | (Some(Factory::Worker), "start")
        )
    })
}
fn strict<'a>(ctx: &TsCtx<'a>, opts: Option<&'a Expression<'a>>) -> bool {
    let Some(p) = opts
        .and_then(|e| super::values::properties(ctx, e))
        .and_then(|p| p.named.get("onUnhandledRequest").copied())
    else {
        return false;
    };
    p.kind == oxc::ast::ast::PropertyKind::Init
        && (super::values::literal(ctx, &p.value) == Some("error")
            || super::values::resolve(ctx, &p.value).is_function())
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    super::values::tracked_uses::report(ctx, RULE.id, findings);
    for node in ctx.semantic.nodes().iter() {
        let Some(c) = super::values::site(ctx, node.kind()) else {
            continue;
        };
        let inv = c.call;
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
