use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-widen-then-assert",
    family: Family::Types,
    provenance: "Port of anti-slop no-widen-then-assert.",
    fix: "Keep the precise type from initialization through use. Parse boundary input once.",
    run: Run::Ts(check),
};

use oxc::{
    ast::{
        AstKind,
        ast::{
            BindingPattern, Expression, IdentifierReference, TSSignature, TSType, TSTypeName,
            TSTypeOperatorOperator, VariableDeclarationKind,
        },
    },
    span::GetSpan,
    syntax::node::NodeId,
};
use std::collections::HashSet;

fn unparen<'a>(mut ty: &'a TSType<'a>) -> &'a TSType<'a> {
    while let TSType::TSParenthesizedType(n) = ty {
        ty = &n.type_annotation;
    }
    ty
}

fn reference<'a>(ty: &'a TSType<'a>) -> Option<(&'a str, &'a [TSType<'a>])> {
    let TSType::TSTypeReference(r) = unparen(ty) else {
        return None;
    };
    let TSTypeName::IdentifierReference(id) = &r.type_name else {
        return None;
    };
    Some((
        id.name.as_str(),
        r.type_arguments
            .as_ref()
            .map_or(&[], |a| a.params.as_slice()),
    ))
}

fn top(ty: &TSType<'_>) -> bool {
    matches!(
        unparen(ty),
        TSType::TSUnknownKeyword(_) | TSType::TSAnyKeyword(_)
    )
}

fn broad_key(ty: &TSType<'_>) -> bool {
    match unparen(ty) {
        TSType::TSStringKeyword(_) | TSType::TSNumberKeyword(_) | TSType::TSSymbolKeyword(_) => {
            true
        }
        TSType::TSUnionType(n) => n.types.iter().all(broad_key),
        _ => matches!(reference(ty), Some(("PropertyKey", _))),
    }
}

fn record(ty: &TSType<'_>, broad: bool) -> bool {
    if let TSType::TSTypeLiteral(n) = unparen(ty) {
        return if broad {
            matches!(n.members.as_slice(),[TSSignature::TSIndexSignature(i)] if broad_key(&i.parameter.type_annotation.type_annotation) && top(&i.type_annotation.type_annotation))
        } else {
            n.members
                .iter()
                .any(|m| !matches!(m, TSSignature::TSIndexSignature(_)))
        };
    }
    match reference(ty) {
        Some(("Readonly", [inner, ..])) => record(inner, broad),
        Some(("Record", [key, value])) => {
            if broad {
                broad_key(key) && top(value)
            } else {
                !top(value)
            }
        }
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Broad {
    Top,
    Object,
    Record,
}

fn broad(ty: &TSType<'_>) -> Option<Broad> {
    if top(ty) {
        Some(Broad::Top)
    } else if matches!(unparen(ty), TSType::TSObjectKeyword(_)) {
        Some(Broad::Object)
    } else {
        record(ty, true).then_some(Broad::Record)
    }
}

fn object(ty: &TSType<'_>) -> bool {
    match unparen(ty) {
        TSType::TSArrayType(_)
        | TSType::TSConstructorType(_)
        | TSType::TSFunctionType(_)
        | TSType::TSMappedType(_)
        | TSType::TSObjectKeyword(_)
        | TSType::TSTupleType(_) => true,
        TSType::TSTypeLiteral(n) => !n.members.is_empty(),
        TSType::TSIntersectionType(n) => n.types.iter().all(object),
        TSType::TSTypeOperatorType(n) => {
            n.operator == TSTypeOperatorOperator::Readonly && object(&n.type_annotation)
        }
        _ => false,
    }
}

fn assertion<'a>(expr: &'a Expression<'a>) -> Option<(&'a Expression<'a>, &'a TSType<'a>)> {
    match expr.without_parentheses() {
        Expression::TSAsExpression(n) => Some((&n.expression, &n.type_annotation)),
        Expression::TSTypeAssertion(n) => Some((&n.expression, &n.type_annotation)),
        _ => None,
    }
}

fn boundary(ctx: &TsCtx<'_>, id: NodeId) -> Option<NodeId> {
    let nodes = ctx.semantic.nodes();
    nodes.ancestor_ids(id).find(|p| {
        matches!(
            nodes.kind(*p),
            AstKind::ArrowFunctionExpression(_) | AstKind::Function(_)
        )
    })
}

pub(super) fn declaration(ctx: &TsCtx<'_>, ident: &IdentifierReference<'_>) -> Option<NodeId> {
    let scopes = ctx.semantic.scoping();
    let symbol = scopes
        .get_reference(ident.reference_id.get()?)
        .symbol_id()?;
    Some(scopes.symbol_declaration(symbol))
}

pub(super) fn immutable(ctx: &TsCtx<'_>, ident: &IdentifierReference<'_>, id: NodeId) -> bool {
    let scopes = ctx.semantic.scoping();
    matches!(ctx.semantic.nodes().parent_kind(id),AstKind::VariableDeclaration(v) if v.kind == VariableDeclarationKind::Const)
        && ident
            .reference_id
            .get()
            .and_then(|r| scopes.get_reference(r).symbol_id())
            .is_some_and(|s| scopes.get_resolved_references(s).all(|r| !r.is_write()))
}

#[derive(Clone, Copy)]
enum Evidence<'a> {
    Value,
    Annotated(&'a TSType<'a>),
}

fn evidence<'a>(
    ctx: &'a TsCtx<'a>,
    mut expr: &'a Expression<'a>,
    owner: Option<NodeId>,
    mut visited: HashSet<NodeId>,
) -> Option<Evidence<'a>> {
    loop {
        let value = expr.without_parentheses();
        if let Some((_, ty)) = assertion(value) {
            return broad(ty).is_none().then_some(Evidence::Annotated(ty));
        }
        if value.is_literal()
            || matches!(
                value,
                Expression::TemplateLiteral(_)
                    | Expression::ArrayExpression(_)
                    | Expression::ArrowFunctionExpression(_)
                    | Expression::ClassExpression(_)
                    | Expression::FunctionExpression(_)
                    | Expression::NewExpression(_)
                    | Expression::ObjectExpression(_)
            )
        {
            return Some(Evidence::Value);
        }
        let Expression::Identifier(ident) = value else {
            return None;
        };
        let id = declaration(ctx, ident)?;
        if !visited.insert(id) || boundary(ctx, id) != owner {
            return None;
        }
        let kind = ctx.semantic.nodes().kind(id);
        let annotation = match kind {
            AstKind::VariableDeclarator(v)
                if matches!(v.id, BindingPattern::BindingIdentifier(_)) =>
            {
                v.type_annotation.as_deref()
            }
            AstKind::FormalParameter(p)
                if matches!(p.pattern, BindingPattern::BindingIdentifier(_)) =>
            {
                p.type_annotation.as_deref()
            }
            _ => None,
        };
        if let Some(a) = annotation {
            return broad(&a.type_annotation)
                .is_none()
                .then_some(Evidence::Annotated(&a.type_annotation));
        }
        let AstKind::VariableDeclarator(v) = kind else {
            return None;
        };
        if !immutable(ctx, ident, id) {
            return None;
        }
        expr = v.init.as_ref()?;
    }
}

fn narrower(ctx: &TsCtx<'_>, kind: Broad, original: Evidence<'_>, target: &TSType<'_>) -> bool {
    if broad(target).is_some() {
        return false;
    }
    if let Evidence::Annotated(old) = original {
        let normalized = |ty: &TSType<'_>| {
            unparen(ty)
                .span()
                .source_text(ctx.text)
                .split_whitespace()
                .collect::<String>()
        };
        if normalized(old) == normalized(target) {
            return true;
        }
    }
    match kind {
        Broad::Top => true,
        Broad::Object => object(target),
        Broad::Record => record(target, false),
    }
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    for (id, node) in nodes.iter_enumerated() {
        let (expr, target) = match node.kind() {
            AstKind::TSAsExpression(n) => (&n.expression, &n.type_annotation),
            AstKind::TSTypeAssertion(n) => (&n.expression, &n.type_annotation),
            _ => continue,
        };
        let Expression::Identifier(ident) = expr.without_parentheses() else {
            continue;
        };
        let Some(binding) = declaration(ctx, ident) else {
            continue;
        };
        let AstKind::VariableDeclarator(v) = nodes.kind(binding) else {
            continue;
        };
        let owner = boundary(ctx, binding);
        if !matches!(v.id, BindingPattern::BindingIdentifier(_))
            || !immutable(ctx, ident, binding)
            || node.span().start <= v.span.end
            || boundary(ctx, id) != owner
        {
            continue;
        }
        let Some(init) = v.init.as_ref() else {
            continue;
        };
        let asserted = assertion(init).and_then(|(e, t)| Some((e, broad(t)?)));
        let Some(kind) = v
            .type_annotation
            .as_ref()
            .and_then(|a| broad(&a.type_annotation))
            .or_else(|| asserted.map(|(_, k)| k))
        else {
            continue;
        };
        if let Some(original) = evidence(
            ctx,
            asserted.map_or(init, |(e, _)| e),
            owner,
            HashSet::from([binding]),
        ) && narrower(ctx, kind, original, target)
        {
            findings.report(node.span().start as usize, None);
        }
    }
}
