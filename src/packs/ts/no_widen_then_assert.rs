use super::dictionary_types::Resolution;
use super::function_types::{
    bindings::{ConstBindings, symbol},
    expressions::{assertion, known_value},
};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, TSSignature, TSType, TSTypeName, TSTypeOperatorOperator},
    },
    span::GetSpan,
    syntax::node::NodeId,
};
use std::ops::ControlFlow::{Break, Continue};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-widen-then-assert",
    family: Family::Types,
    provenance: "Port of anti-slop no-widen-then-assert.",
    fix: "Keep the precise type from initialization through use. Parse boundary input once.",
    run: Run::Ts(check),
};

fn reference<'a>(ctx: &TsCtx<'_>, ty: &'a TSType<'a>) -> Option<(&'a str, &'a [TSType<'a>])> {
    let TSType::TSTypeReference(r) = ty.without_parenthesized() else {
        return None;
    };
    let TSTypeName::IdentifierReference(id) = &r.type_name else {
        return None;
    };
    if symbol(ctx, id).is_some() {
        return None;
    }
    Some((
        id.name.as_str(),
        r.type_arguments
            .as_ref()
            .map_or(&[], |a| a.params.as_slice()),
    ))
}

fn top(ty: &TSType<'_>) -> bool {
    matches!(
        ty.without_parenthesized(),
        TSType::TSUnknownKeyword(_) | TSType::TSAnyKeyword(_)
    )
}

fn broad_key(ctx: &TsCtx<'_>, ty: &TSType<'_>) -> bool {
    match ty.without_parenthesized() {
        TSType::TSStringKeyword(_) | TSType::TSNumberKeyword(_) | TSType::TSSymbolKeyword(_) => {
            true
        }
        TSType::TSUnionType(n) => n.types.iter().all(|ty| broad_key(ctx, ty)),
        _ => matches!(reference(ctx, ty), Some(("PropertyKey", _))),
    }
}

fn broad_index(ctx: &TsCtx<'_>, member: &TSSignature<'_>) -> bool {
    let TSSignature::TSIndexSignature(index) = member else {
        return false;
    };
    broad_key(ctx, &index.parameter.type_annotation.type_annotation)
        && top(&index.type_annotation.type_annotation)
}

fn broad_record(ctx: &TsCtx<'_>, ty: &TSType<'_>) -> bool {
    let mut ty = ty;
    while let Some(("Readonly", [inner, ..])) = reference(ctx, ty) {
        ty = inner;
    }
    if let TSType::TSTypeLiteral(n) = ty.without_parenthesized() {
        return matches!(n.members.as_slice(), [member] if broad_index(ctx, member));
    }
    match reference(ctx, ty) {
        Some(("Record", [key, value])) => broad_key(ctx, key) && top(value),
        _ => false,
    }
}

fn narrow_record(ctx: &TsCtx<'_>, ty: &TSType<'_>) -> bool {
    let mut ty = ty;
    while let Some(("Readonly", [inner, ..])) = reference(ctx, ty) {
        ty = inner;
    }
    if let TSType::TSTypeLiteral(n) = ty.without_parenthesized() {
        return n
            .members
            .iter()
            .any(|m| !matches!(m, TSSignature::TSIndexSignature(_)));
    }
    match reference(ctx, ty) {
        Some(("Record", [_, value])) => !top(value),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Broad {
    Top,
    Object,
    Record,
}

fn broad<'a>(resolver: &Resolution, ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> Option<Broad> {
    let ty = resolver.plain_alias(ctx, ty);
    if top(ty) {
        Some(Broad::Top)
    } else if matches!(ty.without_parenthesized(), TSType::TSObjectKeyword(_)) {
        Some(Broad::Object)
    } else {
        broad_record(ctx, ty).then_some(Broad::Record)
    }
}

fn object(ty: &TSType<'_>) -> bool {
    match ty.without_parenthesized() {
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

fn boundary(ctx: &TsCtx<'_>, id: NodeId) -> Option<NodeId> {
    let nodes = ctx.semantic.nodes();
    nodes.ancestor_ids(id).find(|p| {
        matches!(
            nodes.kind(*p),
            AstKind::ArrowFunctionExpression(_) | AstKind::Function(_)
        )
    })
}

#[derive(Clone, Copy)]
enum Evidence<'a> {
    Value,
    Annotated(&'a TSType<'a>),
}

fn evidence<'a>(
    resolver: &Resolution,
    ctx: &'a TsCtx<'a>,
    expr: &'a Expression<'a>,
    owner: Option<NodeId>,
    bindings: &mut ConstBindings<Evidence<'a>>,
) -> Option<Evidence<'a>> {
    bindings.walk(
        ctx,
        expr,
        owner,
        |expr| {
            let value = expr.without_parentheses();
            if let Some((_, ty)) = assertion(ctx.semantic.nodes().kind(value.node_id())) {
                return Break(
                    broad(resolver, ctx, ty)
                        .is_none()
                        .then_some(Evidence::Annotated(ty)),
                );
            }
            if let Expression::Identifier(i) = value {
                Continue(i)
            } else {
                Break(known_value(value).then_some(Evidence::Value))
            }
        },
        |id, kind| {
            if boundary(ctx, id) != owner {
                return Break(None);
            }
            let annotation = super::function_types::direct_binding(kind).and_then(|b| b.annotation);
            match annotation {
                Some(a) => Break(
                    broad(resolver, ctx, &a.type_annotation)
                        .is_none()
                        .then_some(Evidence::Annotated(&a.type_annotation)),
                ),
                None => Continue(()),
            }
        },
    )
}

fn narrower<'a>(
    resolver: &Resolution,
    ctx: &TsCtx<'a>,
    kind: Broad,
    original: Evidence<'a>,
    target: &'a TSType<'a>,
) -> bool {
    let target = resolver.plain_alias(ctx, target);
    if broad(resolver, ctx, target).is_some() {
        return false;
    }
    if let Evidence::Annotated(old) = original {
        let normalized = |ty: &TSType<'_>| {
            ty.without_parenthesized()
                .span()
                .source_text(ctx.text)
                .split_whitespace()
                .collect::<String>()
        };
        if normalized(resolver.plain_alias(ctx, old)) == normalized(target) {
            return true;
        }
    }
    match kind {
        Broad::Top => true,
        Broad::Object => object(target),
        Broad::Record => narrow_record(ctx, target),
    }
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    check_with_resolution(ctx, findings, &Resolution::default());
}

pub(super) fn check_with_resolution(
    ctx: &TsCtx<'_>,
    findings: &mut Findings,
    resolver: &Resolution,
) {
    let nodes = ctx.semantic.nodes();
    let mut bindings = ConstBindings::default();
    for (id, node) in nodes.iter_enumerated() {
        let Some((expr, target)) = assertion(node.kind()) else {
            continue;
        };
        let Expression::Identifier(ident) = expr.without_parentheses() else {
            continue;
        };
        let Some(symbol) = symbol(ctx, ident) else {
            continue;
        };
        let Some(binding) = super::function_types::bindings::value_declarations(ctx, symbol).next()
        else {
            continue;
        };
        let AstKind::VariableDeclarator(v) = nodes.kind(binding) else {
            continue;
        };
        let owner = boundary(ctx, binding);
        if !bindings.immutable(ctx, symbol, binding)
            || node.span().start <= v.span.end
            || boundary(ctx, id) != owner
        {
            continue;
        }
        let Some(direct) = super::function_types::direct_binding(nodes.kind(binding)) else {
            continue;
        };
        let Some(init) = direct.initializer else {
            continue;
        };
        let asserted = assertion(nodes.kind(init.without_parentheses().node_id()))
            .and_then(|(e, t)| Some((e, broad(resolver, ctx, t)?)));
        let Some(kind) = direct
            .annotation
            .and_then(|a| broad(resolver, ctx, &a.type_annotation))
            .or_else(|| asserted.map(|(_, k)| k))
        else {
            continue;
        };
        if let Some(original) = evidence(
            resolver,
            ctx,
            asserted.map_or(init, |(e, _)| e),
            owner,
            &mut bindings,
        ) && !matches!(original,Evidence::Annotated(t) if direct.annotation.is_some_and(|a|a.type_annotation.node_id()==t.node_id()))
            && narrower(resolver, ctx, kind, original, target)
        {
            findings.report(node.span().start as usize, None);
        }
    }
}
