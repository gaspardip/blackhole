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
use std::ops::ControlFlow::{Break, Continue};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-widen-then-assert",
    family: Family::Types,
    provenance: "Port of anti-slop no-widen-then-assert.",
    fix: "Keep the precise type from initialization through use. Parse boundary input once.",
    run: Run::Ts(check),
};

fn reference<'a>(ctx: &TsCtx<'_>, ty: &'a TSType<'a>) -> Option<(&'a str, &'a [TSType<'a>])> {
    let TSType::TSTypeReference(r) = super::function_types::unparen(ty) else {
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
        super::function_types::unparen(ty),
        TSType::TSUnknownKeyword(_) | TSType::TSAnyKeyword(_)
    )
}

fn broad_key(ctx: &TsCtx<'_>, ty: &TSType<'_>) -> bool {
    let mut pending = vec![ty];
    while let Some(key) = pending.pop() {
        match super::function_types::unparen(key) {
            TSType::TSStringKeyword(_)
            | TSType::TSNumberKeyword(_)
            | TSType::TSSymbolKeyword(_) => {}
            TSType::TSUnionType(n) => pending.extend(&n.types),
            _ if matches!(reference(ctx, key), Some(("PropertyKey", _))) => {}
            _ => return false,
        }
    }
    true
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
    if let TSType::TSTypeLiteral(n) = super::function_types::unparen(ty) {
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
    if let TSType::TSTypeLiteral(n) = super::function_types::unparen(ty) {
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

fn broad(ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Option<Broad> {
    if top(ty) {
        Some(Broad::Top)
    } else if matches!(
        super::function_types::unparen(ty),
        TSType::TSObjectKeyword(_)
    ) {
        Some(Broad::Object)
    } else {
        broad_record(ctx, ty).then_some(Broad::Record)
    }
}

fn object(ty: &TSType<'_>) -> bool {
    let mut pending = vec![ty];
    while let Some(ty) = pending.pop() {
        match super::function_types::unparen(ty) {
            TSType::TSArrayType(_)
            | TSType::TSConstructorType(_)
            | TSType::TSFunctionType(_)
            | TSType::TSMappedType(_)
            | TSType::TSObjectKeyword(_)
            | TSType::TSTupleType(_) => {}
            TSType::TSTypeLiteral(n) if !n.members.is_empty() => {}
            TSType::TSIntersectionType(n) => pending.extend(&n.types),
            TSType::TSTypeOperatorType(n) if n.operator == TSTypeOperatorOperator::Readonly => {
                pending.push(&n.type_annotation);
            }
            _ => return false,
        }
    }
    true
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
    bindings: &mut ConstBindings<Evidence<'a>>,
) -> Option<Evidence<'a>> {
    bindings.walk(
        ctx,
        expr,
        owner,
        |expr| {
            let value = expr.without_parentheses();
            if let Some((_, ty)) = assertion(ctx.semantic.nodes().kind(value.node_id())) {
                return Break(broad(ctx, ty).is_none().then_some(Evidence::Annotated(ty)));
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
                AstKind::FormalParameterRest(p)
                    if matches!(p.rest.argument, BindingPattern::BindingIdentifier(_)) =>
                {
                    p.type_annotation.as_deref()
                }
                _ => None,
            };
            match annotation {
                Some(a) => Break(
                    broad(ctx, &a.type_annotation)
                        .is_none()
                        .then_some(Evidence::Annotated(&a.type_annotation)),
                ),
                None => Continue(()),
            }
        },
    )
}

fn narrower(ctx: &TsCtx<'_>, kind: Broad, original: Evidence<'_>, target: &TSType<'_>) -> bool {
    if broad(ctx, target).is_some() {
        return false;
    }
    if let Evidence::Annotated(old) = original {
        let normalized = |ty: &TSType<'_>| {
            super::function_types::unparen(ty)
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
        Broad::Record => narrow_record(ctx, target),
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
        let Some(binding) = super::function_types::bindings::value_declarations(ctx, symbol).next()
        else {
            continue;
        };
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
            .and_then(|(e, t)| Some((e, broad(ctx, t)?)));
        let Some(kind) = v
            .type_annotation
            .as_ref()
            .and_then(|a| broad(ctx, &a.type_annotation))
            .or_else(|| asserted.map(|(_, k)| k))
        else {
            continue;
        };
        if let Some(original) =
            evidence(ctx, asserted.map_or(init, |(e, _)| e), owner, &mut bindings)
            && !matches!(original,Evidence::Annotated(t) if v.type_annotation.as_ref().is_some_and(|a|a.type_annotation.node_id()==t.node_id()))
            && narrower(ctx, kind, original, target)
        {
            findings.report(node.span().start as usize, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{broad_key, broad_record, narrow_record, object};
    use crate::{
        engine::{Family, Findings, Rule, Run, TsCtx, run},
        packs::ts::function_types::contains_unknown,
    };
    use oxc::{
        allocator::{Allocator, ArenaVec},
        ast::{
            ast::{TSType, TSTypeName, TSTypeParameterInstantiation},
            builder::AstBuilder,
        },
        span::SPAN,
    };

    fn inspect(ctx: &TsCtx<'_>, findings: &mut Findings) {
        let arena = Allocator::default();
        let b = AstBuilder::new(&arena);
        let mut input = TSType::new_ts_unknown_keyword(SPAN, &b);
        let mut key = TSType::new_ts_string_keyword(SPAN, &b);
        let mut shape = TSType::new_ts_object_keyword(SPAN, &b);
        let wrap = |name, arg| {
            TSType::new_ts_type_reference(
                SPAN,
                TSTypeName::new_identifier_reference(SPAN, name, &b),
                Some(TSTypeParameterInstantiation::boxed(
                    SPAN,
                    ArenaVec::from_array_in([arg], &b),
                    &b,
                )),
                &b,
            )
        };
        let mut record = TSType::new_ts_type_reference(
            SPAN,
            TSTypeName::new_identifier_reference(SPAN, "Record", &b),
            Some(TSTypeParameterInstantiation::boxed(
                SPAN,
                ArenaVec::from_array_in(
                    [
                        TSType::new_ts_string_keyword(SPAN, &b),
                        TSType::new_ts_unknown_keyword(SPAN, &b),
                    ],
                    &b,
                ),
                &b,
            )),
            &b,
        );
        for _ in 0..10_000 {
            input = TSType::new_ts_parenthesized_type(
                SPAN,
                TSType::new_ts_union_type(SPAN, ArenaVec::from_array_in([input], &b), &b),
                &b,
            );
            key = TSType::new_ts_union_type(SPAN, ArenaVec::from_array_in([key], &b), &b);
            shape =
                TSType::new_ts_intersection_type(SPAN, ArenaVec::from_array_in([shape], &b), &b);
            record = wrap("Readonly", record);
        }
        assert!(contains_unknown(&input));
        assert!(broad_key(ctx, &key));
        assert!(object(&shape));
        assert!(broad_record(ctx, &record));
        assert!(!narrow_record(ctx, &record));
        findings.report(0, Some("completed"));
    }

    #[test]
    fn deeply_nested_type_nodes_are_walked_without_the_parser() {
        let rule = Rule {
            id: "test/depth",
            family: Family::Types,
            provenance: "test",
            fix: "test",
            run: Run::Ts(inspect),
        };
        let hits = run(&[rule], [("depth.ts", b"".as_slice())]);
        assert_eq!(
            hits.iter().map(|h| h.note.as_deref()).collect::<Vec<_>>(),
            [Some("completed")]
        );
    }
}
