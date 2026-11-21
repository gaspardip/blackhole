use super::{
    test_context::returned_values,
    values::{Origin, declaration, origin_path, resolve},
};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{ArrowFunctionBody, Expression, ObjectProperty},
    },
    span::GetSpan,
};
use std::collections::HashSet;

pub(super) fn config(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut parts = name.split('.');
    parts.next() == Some("vitest")
        && matches!(
            parts.next_back(),
            Some("js" | "cjs" | "mjs" | "ts" | "cts" | "mts")
        )
        && parts.next_back() == Some("config")
        && parts.all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
}

#[derive(Clone, Copy)]
enum ResponseKinds {
    Native,
    WithMsw,
}
pub(super) fn transport_response<'a>(ctx: &TsCtx<'a>, value: &'a Expression<'a>) -> bool {
    raw_response(ctx, value, ResponseKinds::WithMsw)
}
fn config_response<'a>(ctx: &TsCtx<'a>, value: &'a Expression<'a>) -> bool {
    raw_response(ctx, value, ResponseKinds::Native)
}
fn raw_response<'a>(ctx: &TsCtx<'a>, value: &'a Expression<'a>, kinds: ResponseKinds) -> bool {
    let mut pending = vec![value];
    let mut seen = HashSet::new();
    while let Some(expr) = pending.pop() {
        let expr = resolve(ctx, expr);
        if !seen.insert(expr.span()) {
            continue;
        }
        if let Some(call) = super::values::call_expr(expr) {
            let invocation = super::values::invoked(ctx, call);
            match invocation.path.as_deref() {
                Some(["Promise", "resolve"]) => pending.extend(invocation.arg(0)),
                Some(["Response", "json" | "redirect" | "error"]) => return true,
                Some(
                    [
                        "msw",
                        "HttpResponse",
                        "json" | "text" | "html" | "xml" | "arrayBuffer" | "formData" | "error"
                        | "redirect" | "bytes",
                    ],
                ) if matches!(kinds, ResponseKinds::WithMsw) => return true,
                _ => {}
            }
            pending.push(invocation.callee);
            continue;
        }
        match expr {
            Expression::NewExpression(n)
                if origin_path(ctx, &n.callee).is_some_and(|(origin, path)| {
                    origin == Origin::Global && path == ["Response"]
                }) || matches!(kinds, ResponseKinds::WithMsw)
                    && super::values::path(ctx, &n.callee)
                        .is_some_and(|p| p == ["msw", "HttpResponse"]) =>
            {
                return true;
            }
            Expression::Identifier(id) => {
                if let Some(AstKind::Function(f)) = declaration(ctx, id) {
                    returned_values(ctx, f.node_id.get(), &mut pending);
                }
            }
            Expression::FunctionExpression(f) => {
                returned_values(ctx, f.node_id.get(), &mut pending);
            }
            Expression::ArrowFunctionExpression(f) => match &f.body {
                ArrowFunctionBody::FunctionBody(_) => {
                    returned_values(ctx, f.node_id.get(), &mut pending);
                }
                body => pending.extend(body.as_expression()),
            },
            _ => pending.extend(super::values::alternatives(expr)),
        }
    }
    false
}

pub(super) fn replacement<'a>(ctx: &TsCtx<'a>, property: &'a ObjectProperty<'a>) -> bool {
    let key = super::values::key(ctx, &property.key);
    if matches!(key, Some("outboundService") | None) && config_response(ctx, &property.value) {
        return true;
    }
    matches!(key, Some("serviceBindings") | None)
        && super::values::any_property(ctx, &property.value, |p| config_response(ctx, &p.value))
}
