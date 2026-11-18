use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{BindingPattern, Expression, IdentifierReference, MemberExpression, PropertyKey},
    },
    span::GetSpan,
};
use std::collections::HashSet;
pub(super) fn declaration<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<AstKind<'a>> {
    let scope = ctx.semantic.scoping();
    let symbol = scope.get_reference(id.reference_id.get()?).symbol_id()?;
    (!scope.symbol_is_mutated(symbol) && scope.symbol_redeclarations(symbol).is_empty())
        .then(|| ctx.semantic.symbol_declaration(symbol).kind())
}

pub(super) fn initializer<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<&'a Expression<'a>> {
    match declaration(ctx, id)? {
        AstKind::VariableDeclarator(v) if matches!(v.id, BindingPattern::BindingIdentifier(_)) => {
            v.init.as_ref()
        }
        _ => None,
    }
}

pub(super) fn resolve<'a>(ctx: &TsCtx<'a>, mut expr: &'a Expression<'a>) -> &'a Expression<'a> {
    let mut seen = HashSet::new();
    loop {
        expr = expr.get_inner_expression();
        let Expression::Identifier(id) = expr else {
            return expr;
        };
        if !seen.insert(id.span.start) {
            return expr;
        }
        let Some(next) = initializer(ctx, id) else {
            return expr;
        };
        expr = next;
    }
}

pub(super) fn global(ctx: &TsCtx<'_>, expr: &Expression<'_>, name: &str) -> bool {
    matches!(resolve(ctx,expr), Expression::Identifier(id) if id.name == name && ctx.semantic.is_reference_to_global_variable(id))
}

pub(super) fn literal<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<&'a str> {
    match resolve(ctx, expr) {
        Expression::StringLiteral(s) => s.value.as_str(),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
            t.quasis.first()?.value.cooked?.as_str()
        }
        _ => None,
    }
}

pub(super) fn key<'a>(
    ctx: &TsCtx<'a>,
    key: &'a PropertyKey<'a>,
    computed: bool,
) -> Option<&'a str> {
    if !computed && let PropertyKey::StaticIdentifier(id) = key {
        return Some(id.name.as_str());
    }
    literal(ctx, key.as_expression()?)
}

pub(super) fn member_key<'a>(ctx: &TsCtx<'a>, member: &'a MemberExpression<'a>) -> Option<&'a str> {
    match member {
        MemberExpression::StaticMemberExpression(m) => Some(m.property.name.as_str()),
        MemberExpression::ComputedMemberExpression(m) => literal(ctx, &m.expression),
        MemberExpression::PrivateFieldExpression(_) => None,
    }
}

pub(super) fn path(ctx: &TsCtx<'_>, expr: &Expression<'_>) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    let mut expr = expr;
    loop {
        expr = resolve(ctx, expr);
        if !seen.insert(expr.span()) {
            return None;
        }
        if let Some(member) = expr.as_member_expression() {
            parts.push(member_key(ctx, member)?.to_owned());
            expr = member.object();
            continue;
        }
        let Expression::Identifier(id) = expr else {
            return None;
        };
        if ctx.semantic.is_reference_to_global_variable(id) {
            parts.push(id.name.to_string());
            break;
        }
        let decl = declaration(ctx, id)?;
        if let AstKind::VariableDeclarator(v) = decl
            && let BindingPattern::ObjectPattern(pattern) = &v.id
        {
            let p = pattern.properties.iter().find(|p| {
                p.value
                    .get_binding_identifier()
                    .is_some_and(|b| b.name == id.name)
            })?;
            parts.push(key(ctx, &p.key, p.computed)?.to_owned());
            expr = v.init.as_ref()?;
            continue;
        }
        let symbol = ctx
            .semantic
            .scoping()
            .get_reference(id.reference_id.get()?)
            .symbol_id()?;
        let parent = ctx
            .semantic
            .nodes()
            .parent_kind(ctx.semantic.symbol_declaration(symbol).id());
        let AstKind::ImportDeclaration(import) = parent else {
            return None;
        };
        match decl {
            AstKind::ImportSpecifier(s) => parts.push(s.imported.name().as_str().to_owned()),
            AstKind::ImportDefaultSpecifier(_) => parts.push("default".into()),
            AstKind::ImportNamespaceSpecifier(_) => (),
            _ => return None,
        }
        parts.push(import.source.value.as_str()?.to_owned());
        break;
    }
    parts.reverse();
    Some(parts)
}
