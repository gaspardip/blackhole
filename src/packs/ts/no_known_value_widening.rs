use super::{
    dictionary_types::{Class, Judged, Resolution, report},
    function_types::{
        bindings::{ConstBindings, declaration},
        contains_unknown, direct_binding,
        expressions::{assertion, known_value, parent},
        predicate_subject,
    },
};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{
            AssignmentTarget, CallExpression, Expression, FormalParameters, IdentifierReference,
            TSType, TSTypeAnnotation,
        },
    },
    span::GetSpan,
};
use std::ops::ControlFlow::{self, Break, Continue};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-known-value-widening",
    family: Family::Types,
    provenance: "Port of anti-slop no-known-value-widening.",
    fix: "Keep the inferred type, check the value with satisfies, or name the owning contract.",
    run: Run::Ts(check),
};

type Signature<'a> = (&'a FormalParameters<'a>, Option<&'a TSTypeAnnotation<'a>>);
fn signature(kind: AstKind<'_>) -> Option<Signature<'_>> {
    match kind {
        AstKind::Function(n) => Some((&n.params, n.return_type.as_deref())),
        AstKind::ArrowFunctionExpression(n) => Some((&n.params, n.return_type.as_deref())),
        _ => None,
    }
}

fn local_function<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<Signature<'a>> {
    let mut callee = expr.get_inner_expression();
    if let Expression::Identifier(id) = callee {
        let symbol = super::function_types::bindings::symbol(ctx, id)?;
        let mut declarations = super::function_types::bindings::value_declarations(ctx, symbol);
        let declaration = declarations.next()?;
        if declarations.next().is_some() {
            return None;
        }
        let kind = ctx.semantic.nodes().kind(declaration);
        if let AstKind::VariableDeclarator(v) = kind {
            callee = v.init.as_ref()?.get_inner_expression();
        } else {
            return signature(kind);
        }
    }
    match callee {
        Expression::ArrowFunctionExpression(n) => signature(AstKind::ArrowFunctionExpression(n)),
        Expression::FunctionExpression(n) => signature(AstKind::Function(n)),
        _ => None,
    }
}

fn value_or_binding<'a>(
    expr: &'a Expression<'a>,
) -> ControlFlow<Option<()>, &'a IdentifierReference<'a>> {
    let value = expr.get_inner_expression();
    match value {
        Expression::Identifier(i) => Continue(i),
        _ => Break(
            (known_value(value) || matches!(value, Expression::UnaryExpression(_))).then_some(()),
        ),
    }
}

fn known<'a>(ctx: &TsCtx<'a>, bindings: &mut ConstBindings<()>, expr: &'a Expression<'a>) -> bool {
    bindings
        .walk(ctx, expr, None, value_or_binding, |_, _| Continue(()))
        .is_some()
}

fn guard_argument<'a>(
    resolution: &Resolution,
    ctx: &TsCtx<'a>,
    bindings: &mut ConstBindings<Judged<()>>,
    call: &'a CallExpression<'a>,
) -> Option<(&'a Expression<'a>, Judged<()>)> {
    let (params, ret) = local_function(ctx, &call.callee)?;
    let subject = predicate_subject(ret?)?;
    let index = params.items.iter().position(|p| {
        super::function_types::parameter(AstKind::FormalParameter(p))
            .is_some_and(|(name, a)| name == Some(subject) && contains_unknown(a))
    })?;
    let arg = call.arguments.get(index)?.as_expression()?;
    let evidence = bindings.walk(
        ctx,
        arg,
        None,
        |mut expr| loop {
            expr = expr.without_parentheses();
            if let Some((_, ty)) = assertion(ctx.semantic.nodes().kind(expr.node_id())) {
                return Break(
                    resolution
                        .informative(ctx, ty)
                        .map(|yes| yes.then_some(()))
                        .transpose(),
                );
            }
            match expr {
                Expression::TSNonNullExpression(n) => expr = &n.expression,
                Expression::TSSatisfiesExpression(n) => expr = &n.expression,
                Expression::CallExpression(n) => {
                    return Break(
                        local_function(ctx, &n.callee)
                            .and_then(|(_, r)| r)
                            .and_then(|r| {
                                resolution
                                    .informative(ctx, &r.type_annotation)
                                    .map(|yes| yes.then_some(()))
                                    .transpose()
                            }),
                    );
                }
                _ => return value_or_binding(expr).map_break(|v| v.map(Ok)),
            }
        },
        |_, kind| {
            let annotation = direct_binding(kind).and_then(|b| b.annotation);
            annotation.map_or(Continue(()), |a| {
                Break(
                    resolution
                        .informative(ctx, &a.type_annotation)
                        .map(|yes| yes.then_some(()))
                        .transpose(),
                )
            })
        },
    );
    evidence.map(|answer| (arg, answer))
}

fn typed_flow<'a>(
    ctx: &TsCtx<'a>,
    id: oxc::syntax::node::NodeId,
    kind: AstKind<'a>,
) -> Option<(&'a Expression<'a>, &'a TSType<'a>)> {
    let nodes = ctx.semantic.nodes();
    let (expr, a) = match kind {
        AstKind::ArrowFunctionExpression(n) => (n.get_expression()?, n.return_type.as_deref()?),
        AstKind::VariableDeclarator(_) => {
            let binding = direct_binding(kind)?;
            (binding.initializer?, binding.annotation?)
        }
        AstKind::PropertyDefinition(n) => (n.value.as_ref()?, n.type_annotation.as_deref()?),
        AstKind::AccessorProperty(n) => (n.value.as_ref()?, n.type_annotation.as_deref()?),
        AstKind::ReturnStatement(n) => (
            n.argument.as_ref()?,
            nodes
                .ancestor_ids(id)
                .find_map(|a| signature(nodes.kind(a)))?
                .1?,
        ),
        AstKind::AssignmentExpression(n) if n.operator.is_assign() => {
            let AssignmentTarget::AssignmentTargetIdentifier(i) = &n.left else {
                return None;
            };
            let kind = nodes.kind(declaration(ctx, i)?);
            if !matches!(kind, AstKind::VariableDeclarator(_)) {
                return None;
            }
            let binding = direct_binding(kind)?;
            (&n.right, binding.annotation?)
        }
        kind => {
            let pair = assertion(kind)?;
            return parent(ctx, id)
                .and_then(assertion)
                .is_none()
                .then_some(pair);
        }
    };
    Some((expr, &a.type_annotation))
}

fn loses_evidence<'a>(
    resolution: &Resolution,
    ctx: &TsCtx<'a>,
    bindings: &mut ConstBindings<()>,
    expr: &'a Expression<'a>,
    ty: &'a TSType<'a>,
) -> Judged<bool> {
    Ok(match resolution.target(ctx, ty)? {
        Class::None => false,
        Class::Dictionary if matches!(expr.get_inner_expression(), Expression::ObjectExpression(n) if n.properties.is_empty()) => {
            false
        }
        _ => known(ctx, bindings, expr),
    })
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    check_with_resolution(ctx, findings, &Resolution::default());
}

pub(super) fn check_with_resolution(
    ctx: &TsCtx<'_>,
    findings: &mut Findings,
    resolution: &Resolution,
) {
    let mut bindings = ConstBindings::default();
    // Guard calls use declared types as evidence, so their cached origins belong to a separate walk.
    let mut guard_bindings = ConstBindings::default();
    for (id, node) in ctx.semantic.nodes().iter_enumerated() {
        let candidate = match node.kind() {
            AstKind::CallExpression(call) => {
                guard_argument(resolution, ctx, &mut guard_bindings, call)
                    .map(|(expr, answer)| (expr, answer.map(|()| true)))
            }
            kind => typed_flow(ctx, id, kind).map(|(expr, ty)| {
                (
                    expr,
                    loses_evidence(resolution, ctx, &mut bindings, expr, ty),
                )
            }),
        };
        if let Some((expr, answer)) = candidate {
            report(
                findings,
                expr.without_parentheses().span().start as usize,
                answer,
            );
        }
    }
}
