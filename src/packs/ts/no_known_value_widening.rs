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
    no_widen_then_assert::{declaration, immutable},
};
use oxc::{
    ast::{
        AstKind,
        ast::{
            AssignmentTarget, BindingPattern, Expression, FormalParameters, TSType,
            TSTypeAnnotation, TSTypePredicateName,
        },
    },
    span::GetSpan,
};
use std::collections::HashSet;

use super::function_types::contains_unknown;

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

fn known<'a>(ctx: &TsCtx<'a>, mut expr: &'a Expression<'a>, guard: bool) -> bool {
    let mut seen = HashSet::new();
    loop {
        if guard {
            match expr.without_parentheses() {
                Expression::TSNonNullExpression(n) => {
                    expr = &n.expression;
                    continue;
                }
                Expression::TSSatisfiesExpression(n) => {
                    expr = &n.expression;
                    continue;
                }
                Expression::TSAsExpression(n) => return informative(ctx, &n.type_annotation),
                Expression::TSTypeAssertion(n) => return informative(ctx, &n.type_annotation),
                Expression::CallExpression(n) => {
                    return local_function(ctx, &n.callee)
                        .and_then(|(_, r)| r)
                        .is_some_and(|r| informative(ctx, &r.type_annotation));
                }
                _ => {}
            }
        }
        let value = expr.get_inner_expression();
        let Expression::Identifier(ident) = value else {
            return value.is_literal()
                || matches!(
                    value,
                    Expression::TemplateLiteral(_)
                        | Expression::UnaryExpression(_)
                        | Expression::ArrayExpression(_)
                        | Expression::ArrowFunctionExpression(_)
                        | Expression::ClassExpression(_)
                        | Expression::FunctionExpression(_)
                        | Expression::NewExpression(_)
                        | Expression::ObjectExpression(_)
                );
        };
        let Some(id) = declaration(ctx, ident) else {
            return false;
        };
        if !seen.insert(id) {
            return false;
        }
        let kind = ctx.semantic.nodes().kind(id);
        if guard {
            let a = match kind {
                AstKind::VariableDeclarator(n)
                    if matches!(n.id, BindingPattern::BindingIdentifier(_)) =>
                {
                    n.type_annotation.as_deref()
                }
                AstKind::FormalParameter(n) => n.type_annotation.as_deref(),
                _ => None,
            };
            if let Some(a) = a {
                return informative(ctx, &a.type_annotation);
            }
        }
        let AstKind::VariableDeclarator(v) = kind else {
            return false;
        };
        let Some(init) = &v.init else { return false };
        if !immutable(ctx, ident, id) {
            return false;
        }
        expr = init;
    }
}

fn check<'a>(ctx: &TsCtx<'a>, findings: &mut Findings) {
    let flow = |expr: &'a Expression<'a>, ty: &'a TSType<'a>, findings: &mut Findings| {
        let class = target(ctx, ty);
        if class != Class::None
            && !(class == Class::Dictionary
                && matches!(expr.get_inner_expression(),Expression::ObjectExpression(n) if n.properties.is_empty()))
            && known(ctx, expr, false)
        {
            findings.report(expr.without_parentheses().span().start as usize, None);
        }
    };
    let nodes = ctx.semantic.nodes();
    for (id, node) in nodes.iter_enumerated() {
        let pair = match node.kind() {
            AstKind::ArrowFunctionExpression(n) => n.get_expression().zip(n.return_type.as_deref()),
            AstKind::VariableDeclarator(n)
                if matches!(n.id, BindingPattern::BindingIdentifier(_)) =>
            {
                n.init.as_ref().zip(n.type_annotation.as_deref())
            }
            AstKind::PropertyDefinition(n) => n.value.as_ref().zip(n.type_annotation.as_deref()),
            AstKind::AccessorProperty(n) => n.value.as_ref().zip(n.type_annotation.as_deref()),
            AstKind::ReturnStatement(n) => n.argument.as_ref().zip(
                nodes
                    .ancestor_ids(id)
                    .find_map(|a| signature(nodes.kind(a)))
                    .and_then(|(_, r)| r),
            ),
            AstKind::AssignmentExpression(n) if n.operator.is_assign() => {
                if let AssignmentTarget::AssignmentTargetIdentifier(i) = &n.left
                    && let Some(d) = declaration(ctx, i)
                    && let AstKind::VariableDeclarator(v) = nodes.kind(d)
                {
                    v.type_annotation.as_deref().map(|a| (&n.right, a))
                } else {
                    None
                }
            }
            AstKind::CallExpression(n) => {
                if let Some((params, Some(ret))) = local_function(ctx, &n.callee)
                    && let TSType::TSTypePredicate(pred) = &ret.type_annotation
                    && let TSTypePredicateName::Identifier(name) = &pred.parameter_name
                    && let Some((index, _)) = params.items.iter().enumerate().find(|(_, p)| {
                        p.pattern.get_identifier_name() == Some(name.name)
                            && p.type_annotation
                                .as_ref()
                                .is_some_and(|a| contains_unknown(&a.type_annotation))
                    })
                    && let Some(arg) = n.arguments.get(index).and_then(|a| a.as_expression())
                    && known(ctx, arg, true)
                {
                    findings.report(arg.without_parentheses().span().start as usize, None);
                }
                None
            }
            kind => {
                let (expr, ty) = match kind {
                    AstKind::TSAsExpression(n) => (&n.expression, &n.type_annotation),
                    AstKind::TSTypeAssertion(n) => (&n.expression, &n.type_annotation),
                    _ => continue,
                };
                if !matches!(
                    nodes
                        .ancestor_ids(id)
                        .map(|a| nodes.kind(a))
                        .find(|k| !matches!(k, AstKind::ParenthesizedExpression(_))),
                    Some(AstKind::TSAsExpression(_) | AstKind::TSTypeAssertion(_))
                ) {
                    flow(expr, ty, findings);
                }
                None
            }
        };
        if let Some((expr, a)) = pair {
            flow(expr, &a.type_annotation, findings);
        }
    }
}
