use crate::engine::TsCtx;
use oxc::syntax::symbol::SymbolId;
use oxc::{
    ast::{
        AstKind,
        ast::{BindingPattern, Expression, IdentifierReference, MemberExpression, PropertyKey},
    },
    span::GetSpan,
};
use std::collections::HashSet;

pub(super) fn symbol(ctx: &TsCtx<'_>, id: &IdentifierReference<'_>) -> Option<SymbolId> {
    ctx.semantic
        .scoping()
        .get_reference(id.reference_id.get()?)
        .symbol_id()
}
#[derive(PartialEq)]
pub(super) enum Imported<'a> {
    Named(&'a str),
    Default,
    Namespace,
}

pub(super) fn import_of<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<(&'a str, Imported<'a>)> {
    let decl = ctx.semantic.symbol_declaration(symbol(ctx, id)?);
    let AstKind::ImportDeclaration(import) = ctx.semantic.nodes().parent_kind(decl.id()) else {
        return None;
    };
    let name = match decl.kind() {
        AstKind::ImportSpecifier(s) => Imported::Named(s.imported.name().as_str()),
        AstKind::ImportDefaultSpecifier(_) => Imported::Default,
        AstKind::ImportNamespaceSpecifier(_) => Imported::Namespace,
        _ => return None,
    };
    Some((import.source.value.as_str()?, name))
}
pub(super) fn destructured<'a>(
    ctx: &TsCtx<'a>,
    pattern: &'a oxc::ast::ast::ObjectPattern<'a>,
    matches: impl Fn(&oxc::ast::ast::BindingIdentifier<'a>) -> bool,
) -> Option<&'a str> {
    let property = pattern
        .properties
        .iter()
        .find(|p| p.value.get_binding_identifier().is_some_and(&matches))?;
    key(ctx, &property.key, property.computed)
}

pub(super) fn declaration<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<AstKind<'a>> {
    let scope = ctx.semantic.scoping();
    let symbol = symbol(ctx, id)?;
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

pub(super) fn alternatives<'a>(
    expr: &'a Expression<'a>,
) -> impl Iterator<Item = &'a Expression<'a>> {
    let branches = match expr.get_inner_expression() {
        Expression::AwaitExpression(a) => [Some(&a.argument), None],
        Expression::ConditionalExpression(c) => [Some(&c.consequent), Some(&c.alternate)],
        Expression::LogicalExpression(l) => [Some(&l.left), Some(&l.right)],
        Expression::SequenceExpression(s) => [s.expressions.last(), None],
        _ => [None, None],
    };
    branches.into_iter().flatten()
}

pub(super) fn resolves_to_global(ctx: &TsCtx<'_>, expr: &Expression<'_>, name: &str) -> bool {
    let Expression::Identifier(id) = resolve(ctx, expr) else {
        return false;
    };
    id.name == name && ctx.semantic.is_reference_to_global_variable(id)
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
    let expr = key.as_expression()?;
    if let Expression::NumericLiteral(n) = expr {
        return n.raw.as_ref().map(oxc::str::Str::as_str);
    }
    literal(ctx, expr)
}

pub(super) fn member_key<'a>(ctx: &TsCtx<'a>, member: &'a MemberExpression<'a>) -> Option<&'a str> {
    match member {
        MemberExpression::StaticMemberExpression(m) => Some(m.property.name.as_str()),
        MemberExpression::ComputedMemberExpression(m) => literal(ctx, &m.expression),
        MemberExpression::PrivateFieldExpression(_) => None,
    }
}

pub(super) fn path<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<Vec<&'a str>> {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    let mut expr = expr;
    loop {
        expr = resolve(ctx, expr);
        if !seen.insert(expr.span()) {
            return None;
        }
        if let Some(member) = expr.get_member_expr() {
            parts.push(member_key(ctx, member)?);
            expr = member.object();
            continue;
        }
        let Expression::Identifier(id) = expr else {
            return None;
        };
        if ctx.semantic.is_reference_to_global_variable(id) {
            parts.push(id.name.as_str());
            break;
        }
        let decl = declaration(ctx, id)?;
        if let AstKind::VariableDeclarator(v) = decl
            && let BindingPattern::ObjectPattern(pattern) = &v.id
        {
            parts.push(destructured(ctx, pattern, |b| b.name == id.name)?);
            expr = v.init.as_ref()?;
            continue;
        }
        let (source, imported) = import_of(ctx, id)?;
        match imported {
            Imported::Named(name) => parts.push(name),
            Imported::Default => parts.push("default"),
            Imported::Namespace => (),
        }
        parts.push(source);
        break;
    }
    parts.reverse();
    Some(parts)
}

pub(super) fn enclosing_functions<'ctx, 'a>(
    ctx: &'ctx TsCtx<'a>,
    id: oxc::syntax::node::NodeId,
) -> impl Iterator<Item = &'ctx oxc::semantic::AstNode<'a>> {
    ctx.semantic.nodes().ancestors(id).filter(|node| {
        matches!(
            node.kind(),
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
        )
    })
}

pub(super) fn parameter_index(
    params: &oxc::ast::ast::FormalParameters<'_>,
    symbol: SymbolId,
) -> Option<usize> {
    params.items.iter().position(|p| {
        p.pattern
            .get_binding_identifier()
            .is_some_and(|id| id.symbol_id.get() == Some(symbol))
    })
}

pub(super) fn callback_call<'a>(
    ctx: &TsCtx<'a>,
    symbol: SymbolId,
    first_parameters: usize,
) -> Option<super::double_mutations::Invocation<'a>> {
    let function = enclosing_functions(ctx, ctx.semantic.symbol_declaration(symbol).id()).next()?;
    let params = match function.kind() {
        AstKind::Function(f) => &f.params,
        AstKind::ArrowFunctionExpression(f) => &f.params,
        _ => return None,
    };
    if parameter_index(params, symbol)? >= first_parameters {
        return None;
    }
    let AstKind::CallExpression(c) = ctx.semantic.nodes().parent_kind(function.id()) else {
        return None;
    };
    let call = super::double_mutations::invoked(ctx, c);
    call.arg(1)
        .is_some_and(|e| e.span() == function.span())
        .then_some(call)
}

pub(super) fn outer<'a>(ctx: &TsCtx<'a>, id: oxc::syntax::node::NodeId) -> Option<AstKind<'a>> {
    ctx.semantic
        .nodes()
        .ancestors(id)
        .map(oxc::semantic::AstNode::kind)
        .find(|kind| {
            !matches!(
                kind,
                AstKind::ParenthesizedExpression(_)
                    | AstKind::ChainExpression(_)
                    | AstKind::TSAsExpression(_)
                    | AstKind::TSSatisfiesExpression(_)
                    | AstKind::TSInstantiationExpression(_)
                    | AstKind::TSNonNullExpression(_)
                    | AstKind::TSTypeAssertion(_)
            )
        })
}
