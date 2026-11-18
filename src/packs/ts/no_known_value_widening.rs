use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-known-value-widening",
    family: Family::Types,
    provenance: "Port of anti-slop no-known-value-widening.",
    fix: "Keep the inferred type, check the value with satisfies, or name the owning contract.",
    run: Run::Ts(check),
};

use super::{
    dictionary_types::{Class, informative, target},
    function_types::{
        bindings::{ConstBindings, declaration},
        contains_unknown,
        expressions::{assertion, known_value, parent},
    },
};
use oxc::{
    ast::{
        AstKind,
        ast::{
            AssignmentTarget, BindingPattern, CallExpression, Expression, FormalParameters,
            IdentifierReference, TSType, TSTypeAnnotation, TSTypePredicateName,
        },
    },
    span::GetSpan,
};
use std::collections::HashSet;

use std::ops::ControlFlow::{self, Break, Continue};

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
        let kind = ctx.semantic.nodes().kind(declaration(ctx, id)?);
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
) -> ControlFlow<Option<bool>, &'a IdentifierReference<'a>> {
    let value = expr.get_inner_expression();
    match value {
        Expression::Identifier(i) => Continue(i),
        _ => Break(Some(
            known_value(value) || matches!(value, Expression::UnaryExpression(_)),
        )),
    }
}

fn known<'a>(ctx: &TsCtx<'a>, bindings: &mut ConstBindings, expr: &'a Expression<'a>) -> bool {
    bindings
        .walk(ctx, expr, HashSet::new(), value_or_binding, |_, _| {
            Continue(())
        })
        .unwrap_or(false)
}

fn guard_argument<'a>(
    ctx: &TsCtx<'a>,
    bindings: &mut ConstBindings,
    call: &'a CallExpression<'a>,
) -> Option<&'a Expression<'a>> {
    let (params, ret) = local_function(ctx, &call.callee)?;
    let TSType::TSTypePredicate(pred) = &ret?.type_annotation else {
        return None;
    };
    let TSTypePredicateName::Identifier(name) = &pred.parameter_name else {
        return None;
    };
    let index = params.items.iter().position(|p| {
        p.pattern.get_identifier_name() == Some(name.name)
            && p.type_annotation
                .as_ref()
                .is_some_and(|a| contains_unknown(&a.type_annotation))
    })?;
    let arg = call.arguments.get(index)?.as_expression()?;
    let known = bindings.walk(
        ctx,
        arg,
        HashSet::new(),
        |mut expr| loop {
            expr = expr.without_parentheses();
            if let Some((_, ty)) = assertion(ctx.semantic.nodes().kind(expr.node_id())) {
                return Break(Some(informative(ctx, ty)));
            }
            match expr {
                Expression::TSNonNullExpression(n) => expr = &n.expression,
                Expression::TSSatisfiesExpression(n) => expr = &n.expression,
                Expression::CallExpression(n) => {
                    return Break(Some(
                        local_function(ctx, &n.callee)
                            .and_then(|(_, r)| r)
                            .is_some_and(|r| informative(ctx, &r.type_annotation)),
                    ));
                }
                _ => return value_or_binding(expr),
            }
        },
        |_, kind| {
            let annotation = match kind {
                AstKind::VariableDeclarator(v)
                    if matches!(v.id, BindingPattern::BindingIdentifier(_)) =>
                {
                    v.type_annotation.as_ref()
                }
                AstKind::FormalParameter(p) => p.type_annotation.as_ref(),
                _ => None,
            };
            annotation.map_or(Continue(()), |a| {
                Break(Some(informative(ctx, &a.type_annotation)))
            })
        },
    )?;
    known.then_some(arg)
}

fn typed_flow<'a>(
    ctx: &TsCtx<'a>,
    id: oxc::syntax::node::NodeId,
    kind: AstKind<'a>,
) -> Option<(&'a Expression<'a>, &'a TSType<'a>)> {
    let nodes = ctx.semantic.nodes();
    let (expr, a) = match kind {
        AstKind::ArrowFunctionExpression(n) => (n.get_expression()?, n.return_type.as_deref()?),
        AstKind::VariableDeclarator(n) if matches!(n.id, BindingPattern::BindingIdentifier(_)) => {
            (n.init.as_ref()?, n.type_annotation.as_deref()?)
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
            let AstKind::VariableDeclarator(v) = nodes.kind(declaration(ctx, i)?) else {
                return None;
            };
            (&n.right, v.type_annotation.as_deref()?)
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

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut bindings = ConstBindings::default();
    for (id, node) in ctx.semantic.nodes().iter_enumerated() {
        let expr=match node.kind(){
            AstKind::CallExpression(call)=>guard_argument(ctx,&mut bindings,call),
            kind=>typed_flow(ctx,id,kind).and_then(|(expr,ty)|{
                let class=target(ctx,ty);
                let empty=matches!(expr.get_inner_expression(),Expression::ObjectExpression(n) if n.properties.is_empty());
                (class!=Class::None && !(class==Class::Dictionary && empty) && known(ctx,&mut bindings,expr)).then_some(expr)
            }),
        };
        if let Some(expr) = expr {
            findings.report(expr.without_parentheses().span().start as usize, None);
        }
    }
}
