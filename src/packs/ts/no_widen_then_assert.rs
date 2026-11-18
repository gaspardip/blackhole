use super::dictionary_types::{Query, Resolution};
use super::function_types::{
    bindings::{ConstBindings, symbol},
    expressions::{assertion, known_value},
};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, TSType},
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

#[derive(Clone, Copy)]
enum Broad {
    Top,
    Object,
    Record,
}

fn broad<'a>(resolver: &Resolution, ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> Option<Broad> {
    if resolver.assertion_shape(ctx, ty, Query::Top) {
        Some(Broad::Top)
    } else if resolver.assertion_shape(ctx, ty, Query::WideObject) {
        Some(Broad::Object)
    } else {
        resolver
            .assertion_shape(ctx, ty, Query::WideRecord)
            .then_some(Broad::Record)
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
        Broad::Object => resolver.assertion_shape(ctx, target, Query::IsObject),
        Broad::Record => resolver.assertion_shape(ctx, target, Query::NarrowRecord),
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
