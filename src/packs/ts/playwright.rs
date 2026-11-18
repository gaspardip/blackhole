use super::{
    double_mutations::invoked,
    values::{declaration, key, member_key, path, resolve},
};
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
fn typed(ctx: &TsCtx<'_>, e: &Expression<'_>, names: &[&str]) -> bool {
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
fn page(ctx: &TsCtx<'_>, initial: &Expression<'_>) -> bool {
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
        match e {
            Expression::CallExpression(c) => {
                if let Some(m) = invoked(ctx, c).callee.get_member_expr()
                    && matches!(
                        member_key(ctx, m),
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
            }
            _ => pending.extend(super::values::alternatives(e)),
        }
    }
    false
}
fn route(ctx: &TsCtx<'_>, initial: &Expression<'_>) -> bool {
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
        .is_some_and(|m| member_key(ctx, m) == Some("route") && page(ctx, m.object()))
}
pub(super) fn canned(ctx: &TsCtx<'_>, receiver: &Expression<'_>, arg: &Expression<'_>) -> bool {
    if !route(ctx, receiver) {
        return false;
    }
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
                                key(ctx, &p.key, p.computed),
                                Some("body" | "json" | "status" | "contentType") | None
                            ) =>
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
