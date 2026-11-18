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
    let Some(sym) = id
        .reference_id
        .get()
        .and_then(|r| ctx.semantic.scoping().get_reference(r).symbol_id())
    else {
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
    let Some(AstKind::ImportSpecifier(s)) = declaration(ctx, name) else {
        return false;
    };
    matches!(nodes.parent_kind(s.node_id.get()),AstKind::ImportDeclaration(i) if i.source.value.as_str().is_some_and(library))
        && names.contains(&s.imported.name().as_str())
}
fn page(ctx: &TsCtx<'_>, initial: &Expression<'_>) -> bool {
    let mut pending = vec![initial];
    let mut seen = HashSet::new();
    while let Some(e) = pending.pop() {
        let e = resolve(ctx, e);
        if !seen.insert(e.span()) {
            continue;
        }
        if typed(ctx,e,&["Page","BrowserContext","Browser","BrowserType"]) || path(ctx,e).is_some_and(|p|matches!(p.as_slice(),[lib,n] if library(lib)&& matches!(n.as_str(),"chromium"|"firefox"|"webkit"))){return true}
        match e {
            Expression::AwaitExpression(a) => pending.push(&a.argument),
            Expression::ConditionalExpression(c) => pending.extend([&c.consequent, &c.alternate]),
            Expression::CallExpression(c) => {
                if let Some(m) = invoked(ctx, c).callee.as_member_expression()
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
            _ => (),
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
    let Some(symbol) = id
        .reference_id
        .get()
        .and_then(|r| ctx.semantic.scoping().get_reference(r).symbol_id())
    else {
        return false;
    };
    let nodes = ctx.semantic.nodes();
    let Some(owner) = nodes
        .ancestors(ctx.semantic.symbol_declaration(symbol).id())
        .find(|n| {
            matches!(
                n.kind(),
                AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
            )
        })
    else {
        return false;
    };
    let params = match owner.kind() {
        AstKind::Function(f) => &f.params,
        AstKind::ArrowFunctionExpression(f) => &f.params,
        _ => return false,
    };
    if params
        .items
        .first()
        .and_then(|p| p.pattern.get_binding_identifier())
        .and_then(|id| id.symbol_id.get())
        != Some(symbol)
    {
        return false;
    }
    let AstKind::CallExpression(c) = nodes.parent_kind(owner.id()) else {
        return false;
    };
    let call = invoked(ctx, c);
    call.arg(1).is_some_and(|e| e.span() == owner.span())
        && call
            .callee
            .as_member_expression()
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
            Expression::LogicalExpression(l) => {
                pending.extend([(&l.left, field), (&l.right, field)]);
            }
            Expression::ConditionalExpression(c) => {
                pending.extend([(&c.consequent, field), (&c.alternate, field)]);
            }
            Expression::ObjectExpression(o) => {
                for p in &o.properties {
                    match p {
                        ObjectPropertyKind::SpreadProperty(s) => pending.push((&s.argument, false)),
                        ObjectPropertyKind::ObjectProperty(p)
                            if matches!(
                                key(ctx, &p.key, p.computed),
                                Some("body" | "json" | "status" | "contentType")
                            ) =>
                        {
                            pending.push((&p.value, true));
                        }
                        ObjectPropertyKind::ObjectProperty(_) => (),
                    }
                }
            }
            _ => (),
        }
    }
    false
}
