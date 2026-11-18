use super::function_types::{
    bindings::{ConstBindings, symbol},
    expressions::{assertion, known_value},
};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{
            BindingPattern, Expression, TSSignature, TSType, TSTypeName, TSTypeOperatorOperator,
        },
    },
    span::GetSpan,
    syntax::node::NodeId,
};
use std::{
    collections::HashSet,
    ops::ControlFlow::{Break, Continue},
};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-widen-then-assert",
    family: Family::Types,
    provenance: "Port of anti-slop no-widen-then-assert.",
    fix: "Keep the precise type from initialization through use. Parse boundary input once.",
    run: Run::Ts(check),
};

fn reference<'a>(ty: &'a TSType<'a>) -> Option<(&'a str, &'a [TSType<'a>])> {
    let TSType::TSTypeReference(r) = ty.without_parenthesized() else {
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
        ty.without_parenthesized(),
        TSType::TSUnknownKeyword(_) | TSType::TSAnyKeyword(_)
    )
}

fn broad_key(ty: &TSType<'_>) -> bool {
    match ty.without_parenthesized() {
        TSType::TSStringKeyword(_) | TSType::TSNumberKeyword(_) | TSType::TSSymbolKeyword(_) => {
            true
        }
        TSType::TSUnionType(n) => n.types.iter().all(broad_key),
        _ => matches!(reference(ty), Some(("PropertyKey", _))),
    }
}

fn broad_index(member: &TSSignature<'_>) -> bool {
    let TSSignature::TSIndexSignature(index) = member else {
        return false;
    };
    broad_key(&index.parameter.type_annotation.type_annotation)
        && top(&index.type_annotation.type_annotation)
}

fn broad_record(ty: &TSType<'_>) -> bool {
    if let TSType::TSTypeLiteral(n) = ty.without_parenthesized() {
        return matches!(n.members.as_slice(), [member] if broad_index(member));
    }
    match reference(ty) {
        Some(("Readonly", [inner, ..])) => broad_record(inner),
        Some(("Record", [key, value])) => broad_key(key) && top(value),
        _ => false,
    }
}

fn narrow_record(ty: &TSType<'_>) -> bool {
    if let TSType::TSTypeLiteral(n) = ty.without_parenthesized() {
        return n
            .members
            .iter()
            .any(|m| !matches!(m, TSSignature::TSIndexSignature(_)));
    }
    match reference(ty) {
        Some(("Readonly", [inner, ..])) => narrow_record(inner),
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

fn broad(ty: &TSType<'_>) -> Option<Broad> {
    if top(ty) {
        Some(Broad::Top)
    } else if matches!(ty.without_parenthesized(), TSType::TSObjectKeyword(_)) {
        Some(Broad::Object)
    } else {
        broad_record(ty).then_some(Broad::Record)
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
    ctx: &'a TsCtx<'a>,
    expr: &'a Expression<'a>,
    owner: Option<NodeId>,
    binding: NodeId,
    bindings: &mut ConstBindings,
) -> Option<Evidence<'a>> {
    bindings.walk(
        ctx,
        expr,
        HashSet::from([binding]),
        |expr| {
            let value = expr.without_parentheses();
            if let Some((_, ty)) = assertion(ctx.semantic.nodes().kind(value.node_id())) {
                return Break(broad(ty).is_none().then_some(Evidence::Annotated(ty)));
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
            match annotation {
                Some(a) => Break(
                    broad(&a.type_annotation)
                        .is_none()
                        .then_some(Evidence::Annotated(&a.type_annotation)),
                ),
                None => Continue(()),
            }
        },
    )
}

fn narrower(ctx: &TsCtx<'_>, kind: Broad, original: Evidence<'_>, target: &TSType<'_>) -> bool {
    if broad(target).is_some() {
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
        if normalized(old) == normalized(target) {
            return true;
        }
    }
    match kind {
        Broad::Top => true,
        Broad::Object => object(target),
        Broad::Record => narrow_record(target),
    }
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
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
        let binding = ctx.semantic.scoping().symbol_declaration(symbol);
        let AstKind::VariableDeclarator(v) = nodes.kind(binding) else {
            continue;
        };
        let owner = boundary(ctx, binding);
        if !matches!(v.id, BindingPattern::BindingIdentifier(_))
            || !bindings.immutable(ctx, symbol, binding)
            || node.span().start <= v.span.end
            || boundary(ctx, id) != owner
        {
            continue;
        }
        let Some(init) = v.init.as_ref() else {
            continue;
        };
        let asserted = assertion(nodes.kind(init.without_parentheses().node_id()))
            .and_then(|(e, t)| Some((e, broad(t)?)));
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
            binding,
            &mut bindings,
        ) && narrower(ctx, kind, original, target)
        {
            findings.report(node.span().start as usize, None);
        }
    }
}
