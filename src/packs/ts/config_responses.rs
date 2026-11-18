use super::values::{declaration, resolve, resolves_to_global};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{ArrowFunctionBody, Expression, ObjectProperty, ObjectPropertyKind},
    },
    span::GetSpan,
    syntax::node::NodeId,
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

fn returns<'a>(ctx: &TsCtx<'a>, function: NodeId, pending: &mut Vec<&'a Expression<'a>>) {
    for (id, node) in ctx.semantic.nodes().iter_enumerated() {
        if let AstKind::ReturnStatement(ret) = node.kind()
            && let Some(arg) = &ret.argument
            && super::values::enclosing_functions(ctx, id)
                .next()
                .is_some_and(|n| n.id() == function)
        {
            pending.push(arg);
        }
    }
}

#[derive(Clone, Copy)]
enum ResponseKinds {
    Native,
    WithMsw,
}
pub(super) fn transport_response(ctx: &TsCtx<'_>, value: &Expression<'_>) -> bool {
    raw_response(ctx, value, ResponseKinds::WithMsw)
}
fn config_response(ctx: &TsCtx<'_>, value: &Expression<'_>) -> bool {
    raw_response(ctx, value, ResponseKinds::Native)
}
fn raw_response(ctx: &TsCtx<'_>, value: &Expression<'_>, kinds: ResponseKinds) -> bool {
    let mut pending = vec![value];
    let mut seen = HashSet::new();
    while let Some(expr) = pending.pop() {
        let expr = resolve(ctx, expr);
        if !seen.insert(expr.span()) {
            continue;
        }
        match expr {
            Expression::NewExpression(n)
                if resolves_to_global(ctx, &n.callee, "Response")
                    || matches!(kinds, ResponseKinds::WithMsw)
                        && super::values::path(ctx, &n.callee)
                            .is_some_and(|p| p == ["msw", "HttpResponse"]) =>
            {
                return true;
            }
            Expression::CallExpression(call) => {
                let invocation = super::double_mutations::invoked(ctx, call);
                let callee = invocation.callee;
                if invocation.path.as_deref() == Some(&["Promise", "resolve"]) {
                    pending.extend(invocation.arg(0));
                }
                if matches!(kinds, ResponseKinds::WithMsw)
                    && let Some(m) = callee.get_member_expr()
                    && super::values::path(ctx, m.object())
                        .is_some_and(|p| p == ["msw", "HttpResponse"])
                    && matches!(
                        super::values::member_key(ctx, m),
                        Some(
                            "json"
                                | "text"
                                | "html"
                                | "xml"
                                | "arrayBuffer"
                                | "formData"
                                | "error"
                                | "redirect"
                                | "bytes"
                        )
                    )
                {
                    return true;
                }
                if let Some(member) = callee.get_member_expr()
                    && matches!(
                        member
                            .static_property_name()
                            .and_then(oxc::str::JSStr::as_str),
                        Some("json" | "redirect" | "error")
                    )
                    && resolves_to_global(ctx, member.object(), "Response")
                {
                    return true;
                }
                pending.push(callee);
            }
            Expression::Identifier(id) => {
                if let Some(AstKind::Function(f)) = declaration(ctx, id) {
                    returns(ctx, f.node_id.get(), &mut pending);
                }
            }
            Expression::FunctionExpression(f) => returns(ctx, f.node_id.get(), &mut pending),
            Expression::ArrowFunctionExpression(f) => match &f.body {
                ArrowFunctionBody::FunctionBody(_) => returns(ctx, f.node_id.get(), &mut pending),
                body => pending.extend(body.as_expression()),
            },
            _ => pending.extend(super::values::alternatives(expr)),
        }
    }
    false
}

pub(super) fn replacement(ctx: &TsCtx<'_>, property: &ObjectProperty<'_>) -> bool {
    let key = super::values::key(ctx, &property.key, property.computed);
    if matches!(key, Some("outboundService") | None) && config_response(ctx, &property.value) {
        return true;
    }
    match key {
        Some("serviceBindings") | None => {
            let mut pending = vec![&property.value];
            let mut seen = HashSet::new();
            while let Some(expr) = pending.pop() {
                let expr = resolve(ctx, expr);
                if !seen.insert(expr.span()) {
                    continue;
                }
                let Expression::ObjectExpression(obj) = expr else {
                    pending.extend(super::values::alternatives(expr));
                    continue;
                };
                for p in &obj.properties {
                    match p {
                        ObjectPropertyKind::SpreadProperty(s) => pending.push(&s.argument),
                        ObjectPropertyKind::ObjectProperty(p) if config_response(ctx, &p.value) => {
                            return true;
                        }
                        ObjectPropertyKind::ObjectProperty(_) => (),
                    }
                }
            }
            false
        }
        _ => false,
    }
}
