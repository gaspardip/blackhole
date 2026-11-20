use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{ArrowFunctionBody, Expression, FormalParameters, Statement},
};

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
    super::values::receiver(ctx, value).is_some_and(|(owner, key)| starts(factory(ctx, owner), key))
}
fn starts(factory: Option<Factory>, key: &str) -> bool {
    matches!(
        (factory, key),
        (Some(Factory::Server), "listen") | (Some(Factory::Worker), "start")
    )
}

fn warning_call<'a>(
    ctx: &TsCtx<'a>,
    params: &FormalParameters<'_>,
    expr: &'a Expression<'a>,
) -> bool {
    use super::values::{Origin, call_expr, invoked, origin_path, receiver, resolve, symbol};
    let Some(call) = call_expr(resolve(ctx, expr)) else {
        return false;
    };
    let callee = invoked(ctx, call).callee;
    if origin_path(ctx, callee).is_some_and(|(origin, path)| {
        origin == Origin::Global && matches!(path.as_slice(), ["console", "warn" | "log" | "info"])
    }) {
        return true;
    }
    let Some((owner, "warning")) = receiver(ctx, callee) else {
        return false;
    };
    let print = params
        .items
        .get(1)
        .and_then(|param| param.pattern.get_binding_identifier())
        .and_then(|id| id.symbol_id.get());
    print.is_some_and(|print| {
        resolve(ctx, owner)
            .get_identifier_reference()
            .and_then(|id| symbol(ctx, id))
            == Some(print)
    })
}

fn empty_or_warning_only<'a>(ctx: &TsCtx<'a>, callback: &'a Expression<'a>) -> bool {
    let (params, body) = match callback {
        Expression::FunctionExpression(f) => (&f.params, f.body.as_deref()),
        Expression::ArrowFunctionExpression(f) => match &f.body {
            ArrowFunctionBody::FunctionBody(body) => (&f.params, Some(&**body)),
            body => {
                return body
                    .as_expression()
                    .is_some_and(|expr| warning_call(ctx, &f.params, expr));
            }
        },
        _ => return false,
    };
    body.is_some_and(|body| {
        body.statements.iter().all(|statement| {
            matches!(statement, Statement::ExpressionStatement(s) if warning_call(ctx, params, &s.expression))
        })
    })
}

fn strict<'a>(ctx: &TsCtx<'a>, opts: Option<&'a Expression<'a>>) -> bool {
    let Some(p) = opts
        .and_then(|e| super::values::properties(ctx, e))
        .and_then(|p| p.named.get("onUnhandledRequest").copied())
    else {
        return false;
    };
    let value = super::values::resolve(ctx, &p.value);
    p.kind == oxc::ast::ast::PropertyKind::Init
        && (super::values::literal(ctx, value) == Some("error")
            || value.is_function() && !empty_or_warning_only(ctx, value))
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
        if super::values::any_value(ctx, object, |e| starts(factory(ctx, e), key))
            && !strict(ctx, inv.arg(0))
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
