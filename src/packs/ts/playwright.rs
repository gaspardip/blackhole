use super::values::{MemberKeys, declaration, invoked, key, path, resolve};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, ObjectPropertyKind, TSType, TSTypeName},
    },
    span::GetSpan,
};
use std::collections::HashSet;
fn library(s: &str) -> bool {
    matches!(s, "@playwright/test" | "playwright" | "playwright-core")
}
fn typed<'a>(ctx: &TsCtx<'a>, e: &'a Expression<'a>, names: &[&str]) -> bool {
    let Expression::Identifier(id) = e else {
        return false;
    };
    let Some(sym) = super::values::symbol(ctx, id) else {
        return false;
    };
    let nodes = ctx.semantic.nodes();
    let decl = ctx.semantic.symbol_declaration(sym);
    let Some(ty) = std::iter::once(decl)
        .chain(nodes.ancestors(decl.id()))
        .find_map(|n| match n.kind() {
            AstKind::FormalParameter(p) => p.type_annotation.as_ref(),
            AstKind::VariableDeclarator(v) => v.type_annotation.as_ref(),
            _ => None,
        })
    else {
        return false;
    };
    let TSType::TSTypeReference(t) = &ty.type_annotation else {
        return false;
    };
    let TSTypeName::IdentifierReference(name) = &t.type_name else {
        return false;
    };
    matches!(declaration(ctx, name), Some(AstKind::ImportSpecifier(_)))
        && super::values::import_of(ctx, name).is_some_and(|(source, imported)| {
            library(source)
                && matches!(imported,super::values::Imported::Named(n) if names.contains(&n))
        })
}
fn browser_engine(parts: &[&str]) -> bool {
    matches!(parts,[source,"chromium"|"firefox"|"webkit"] if library(source))
}
fn page<'a>(ctx: &TsCtx<'a>, initial: &'a Expression<'a>) -> bool {
    let mut pending = vec![initial];
    let mut seen = HashSet::new();
    while let Some(e) = pending.pop() {
        let e = resolve(ctx, e);
        if !seen.insert(e.span()) {
            continue;
        }
        if typed(
            ctx,
            e,
            &["Page", "BrowserContext", "Browser", "BrowserType"],
        ) || path(ctx, e).is_some_and(|p| browser_engine(&p))
        {
            return true;
        }
        if let Some(c) = super::values::call_expr(e) {
            if let Some(m) = invoked(ctx, c).callee.get_member_expr()
                && matches!(
                    MemberKeys(ctx).read(m),
                    Some(
                        "launch"
                            | "launchPersistentContext"
                            | "connect"
                            | "connectOverCDP"
                            | "newContext"
                            | "newPage"
                    )
                )
            {
                pending.push(m.object());
            }

            continue;
        }
        pending.extend(super::values::alternatives(e));
    }
    false
}
fn route<'a>(ctx: &TsCtx<'a>, initial: &'a Expression<'a>) -> bool {
    let e = resolve(ctx, initial);
    if typed(ctx, e, &["Route"]) {
        return true;
    }
    let Expression::Identifier(id) = e else {
        return false;
    };
    let Some(symbol) = super::values::symbol(ctx, id) else {
        return false;
    };
    let Some(call) = super::values::callback_call(ctx, symbol, 1) else {
        return false;
    };
    call.callee
        .get_member_expr()
        .is_some_and(|m| MemberKeys(ctx).read(m) == Some("route") && page(ctx, m.object()))
}
pub(super) fn canned<'a>(
    ctx: &TsCtx<'a>,
    receiver: &'a Expression<'a>,
    arg: &'a Expression<'a>,
) -> bool {
    if !route(ctx, receiver) {
        return false;
    }
    let forwarded = super::values::properties(ctx, arg)
        .and_then(|options| options.named.get("response").copied())
        .is_some_and(|p| forwarded_response(ctx, receiver, &p.value));
    let mut pending = vec![(arg, false)];
    let mut seen = HashSet::new();
    while let Some((e, field)) = pending.pop() {
        let e = resolve(ctx, e);
        if !seen.insert((e.span(), field)) {
            continue;
        }
        if field
            && (e.is_literal()
                || matches!(
                    e,
                    Expression::ObjectExpression(_)
                        | Expression::ArrayExpression(_)
                        | Expression::TemplateLiteral(_)
                ))
        {
            return true;
        }
        match e {
            Expression::ObjectExpression(o) => {
                for p in &o.properties {
                    match p {
                        ObjectPropertyKind::SpreadProperty(s) => pending.push((&s.argument, false)),
                        ObjectPropertyKind::ObjectProperty(p)
                            if matches!(
                                key(ctx, &p.key),
                                Some("body" | "json" | "status") | None
                            ) || !forwarded && key(ctx, &p.key) == Some("contentType") =>
                        {
                            pending.push((&p.value, true));
                        }
                        ObjectPropertyKind::ObjectProperty(_) => (),
                    }
                }
            }
            _ => pending.extend(super::values::alternatives(e).map(|branch| (branch, field))),
        }
    }
    false
}

fn forwarded_response<'a>(
    ctx: &TsCtx<'a>,
    route: &'a Expression<'a>,
    response: &'a Expression<'a>,
) -> bool {
    let Some(route) = resolve(ctx, route)
        .get_identifier_reference()
        .and_then(|id| super::values::symbol(ctx, id))
    else {
        return false;
    };
    let Expression::AwaitExpression(awaited) = resolve(ctx, response) else {
        return false;
    };
    super::values::call_expr(resolve(ctx, &awaited.argument))
        .and_then(|call| super::values::receiver(ctx, invoked(ctx, call).callee))
        .is_some_and(|(owner, method)| {
            method == "fetch"
                && resolve(ctx, owner)
                    .get_identifier_reference()
                    .and_then(|id| super::values::symbol(ctx, id))
                    == Some(route)
        })
}
