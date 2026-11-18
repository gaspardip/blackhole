use super::{
    config_responses::raw_response,
    values::{declaration, initializer, key, path, resolve},
};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, ObjectPropertyKind, Statement, TSType, TSTypeName, TSTypeQueryExprName},
    },
    span::{GetSpan, Span},
};
use std::collections::HashSet;

fn fetch<'a>(ctx: &TsCtx<'a>, mut ty: &'a TSType<'a>) -> bool {
    let mut seen = HashSet::new();
    while seen.insert(ty.span()) {
        match ty {
            TSType::TSTypeQuery(q) => {
                return match &q.expr_name {
                    TSTypeQueryExprName::IdentifierReference(id) => {
                        if ctx.semantic.is_reference_to_global_variable(id) {
                            id.name == "fetch"
                        } else {
                            initializer(ctx,id).and_then(|e|path(ctx,e)).is_some_and(|p|p==["fetch"] || matches!(p.as_slice(),[root,m] if matches!(root.as_str(),"window"|"self"|"global"|"globalThis") && m=="fetch"))
                        }
                    }
                    TSTypeQueryExprName::QualifiedName(n) => {
                        n.right.name == "fetch"
                            && matches!(&n.left,TSTypeName::IdentifierReference(id) if ctx.semantic.is_reference_to_global_variable(id)&&matches!(id.name.as_str(),"window"|"globalThis"|"global"|"self"))
                    }
                    _ => false,
                };
            }
            TSType::TSTypeReference(r) => {
                let TSTypeName::IdentifierReference(id) = &r.type_name else {
                    return false;
                };
                let Some(AstKind::TSTypeAliasDeclaration(a)) = declaration(ctx, id) else {
                    return false;
                };
                ty = &a.type_annotation;
            }
            _ => return false,
        }
    }
    false
}

fn option<'a>(ctx: &TsCtx<'a>, initial: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    enum Item<'a> {
        Object(&'a Expression<'a>),
        Field(&'a oxc::ast::ast::ObjectProperty<'a>),
        Leave(Span),
    }
    let mut pending = vec![Item::Object(initial)];
    let mut ancestors = HashSet::new();
    while let Some(item) = pending.pop() {
        match item {
            Item::Object(e) => {
                let Expression::ObjectExpression(o) = resolve(ctx, e) else {
                    return None;
                };
                if !ancestors.insert(o.span) {
                    return None;
                }
                pending.push(Item::Leave(o.span));
                for p in &o.properties {
                    pending.push(match p {
                        ObjectPropertyKind::SpreadProperty(s) => Item::Object(&s.argument),
                        ObjectPropertyKind::ObjectProperty(p) => Item::Field(p),
                    });
                }
            }
            Item::Field(f) => {
                if key(ctx, &f.key, f.computed)? == "fetch" {
                    return Some(&f.value);
                }
            }
            Item::Leave(s) => {
                ancestors.remove(&s);
            }
        }
    }
    None
}

fn arguments<'a>(
    ctx: &TsCtx<'a>,
    symbol: oxc::syntax::symbol::SymbolId,
    next: &mut Vec<&'a Expression<'a>>,
) {
    let decl = ctx.semantic.symbol_declaration(symbol);
    if let Some(f) = ctx.semantic.nodes().ancestors(decl.id()).find_map(|n| {
        if let AstKind::Function(f) = n.kind() {
            Some(f)
        } else {
            None
        }
    }) && let Some(index) = f.params.items.iter().position(|p| {
        p.pattern
            .get_binding_identifier()
            .is_some_and(|id| id.symbol_id.get() == Some(symbol))
    }) && let Some(fn_id) = f.id.as_ref().and_then(|id| id.symbol_id.get())
    {
        for reference in ctx.semantic.scoping().get_resolved_references(fn_id) {
            if let AstKind::CallExpression(c) =
                ctx.semantic.nodes().parent_kind(reference.node_id())
                && c.callee.span() == ctx.semantic.nodes().get_node(reference.node_id()).span()
                && let Some(arg) = c.arguments.get(index).and_then(|a| a.as_expression())
            {
                next.push(arg);
            }
        }
    }
}
pub(super) fn results<'a>(ctx: &TsCtx<'a>, initial: &'a Expression<'a>) -> Vec<&'a Expression<'a>> {
    let mut pending = vec![(initial, Vec::<String>::new(), HashSet::new())];
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    while let Some((e, mut fields, mut ancestors)) = pending.pop() {
        let e = resolve(ctx, e);
        if !ancestors.insert(e.span()) || !seen.insert((e.span(), fields.clone())) {
            continue;
        }
        let mut next = Vec::new();
        if let Expression::Identifier(id) = e
            && let Some(symbol) = id
                .reference_id
                .get()
                .and_then(|r| ctx.semantic.scoping().get_reference(r).symbol_id())
        {
            let decl = ctx.semantic.symbol_declaration(symbol);
            if let AstKind::VariableDeclarator(v) = decl.kind()
                && let oxc::ast::ast::BindingPattern::ObjectPattern(obj) = &v.id
                && let Some(init) = &v.init
                && let Some(prop) = obj.properties.iter().find(|p| {
                    p.value
                        .get_binding_identifier()
                        .is_some_and(|id| id.symbol_id.get() == Some(symbol))
                })
                && let Some(k) = key(ctx, &prop.key, prop.computed)
            {
                fields.push(k.to_owned());
                pending.push((init, fields, ancestors));
                continue;
            }
            arguments(ctx, symbol, &mut next);
        }
        if let Some(m) = e.as_member_expression()
            && let Some(k) = super::values::member_key(ctx, m)
        {
            fields.push(k.to_owned());
            pending.push((m.object(), fields, ancestors));
            continue;
        }
        if let Expression::ObjectExpression(o) = e
            && let Some(wanted) = fields.last()
        {
            for p in &o.properties {
                match p {
                    ObjectPropertyKind::SpreadProperty(s) => {
                        pending.push((&s.argument, fields.clone(), ancestors.clone()));
                    }
                    ObjectPropertyKind::ObjectProperty(p)
                        if key(ctx, &p.key, p.computed) == Some(wanted) =>
                    {
                        pending.push((
                            &p.value,
                            fields[..fields.len() - 1].to_vec(),
                            ancestors.clone(),
                        ));
                    }
                    ObjectPropertyKind::ObjectProperty(_) => (),
                }
            }
            continue;
        }
        if let Expression::ConditionalExpression(c) = e {
            next.extend([&c.consequent, &c.alternate]);
        }
        if let Expression::CallExpression(c) = e {
            let callee = resolve(ctx, &c.callee);
            let body = match callee {
                Expression::Identifier(id) => match declaration(ctx, id) {
                    Some(AstKind::Function(f)) => f.body.as_deref(),
                    _ => None,
                },
                Expression::FunctionExpression(f) => f.body.as_deref(),
                Expression::ArrowFunctionExpression(f) => {
                    if let Some(value) = f.body.as_expression() {
                        pending.push((value, fields, ancestors));
                        continue;
                    } else if let oxc::ast::ast::ArrowFunctionBody::FunctionBody(b) = &f.body {
                        Some(&**b)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(body) = body {
                next.extend(body.statements.iter().filter_map(|s| match s {
                    Statement::ReturnStatement(r) => r.argument.as_ref(),
                    _ => None,
                }));
            }
        }
        if next.is_empty() && fields.is_empty() {
            results.push(e);
        }
        pending.extend(
            next.into_iter()
                .map(|e| (e, fields.clone(), ancestors.clone())),
        );
    }
    results
}

pub(super) fn report(
    ctx: &TsCtx<'_>,
    kind: AstKind<'_>,
    reported: &mut HashSet<Span>,
) -> Option<usize> {
    let transport = match kind {
        AstKind::VariableDeclarator(v)
            if v.type_annotation
                .as_ref()
                .is_some_and(|a| fetch(ctx, &a.type_annotation)) =>
        {
            v.init.as_ref()?
        }
        AstKind::NewExpression(n)
            if path(ctx, &n.callee).is_some_and(|p| {
                p == ["@knocklabs/node", "default"] || p == ["@knocklabs/node", "Knock"]
            }) =>
        {
            results(ctx, n.arguments.first()?.as_expression()?)
                .into_iter()
                .filter_map(|e| option(ctx, e))
                .find(|e| raw_response(ctx, e, true))?
        }
        _ => return None,
    };
    (raw_response(ctx, transport, true) && reported.insert(resolve(ctx, transport).span()))
        .then_some(transport.span().start as usize)
}
