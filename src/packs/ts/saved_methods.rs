use super::values::symbol;
use super::{
    double_mutations::writes,
    values::{MemberKeys, declaration, invoked, path, resolve},
};
use crate::engine::{Findings, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{AssignmentExpression, Expression, Function, MemberExpression, Statement},
    },
    span::GetSpan,
    syntax::symbol::SymbolId,
};

fn same(ctx: &TsCtx<'_>, left: &MemberExpression<'_>, right: &MemberExpression<'_>) -> bool {
    let (mut left, mut right) = (left, right);
    let mut seen = std::collections::HashSet::new();
    while seen.insert(left.span()) {
        if MemberKeys::Resolve(ctx).read(left).is_none()
            || MemberKeys::Resolve(ctx).read(left) != MemberKeys::Resolve(ctx).read(right)
        {
            return false;
        }
        let (left_object, right_object) =
            (resolve(ctx, left.object()), resolve(ctx, right.object()));
        if left_object.span() == right_object.span() {
            return true;
        }
        if let (Expression::Identifier(first), Expression::Identifier(second)) =
            (left_object, right_object)
        {
            return first.name == second.name && symbol(ctx, first) == symbol(ctx, second);
        }
        let (Some(first), Some(second)) = (
            left_object.get_member_expr(),
            right_object.get_member_expr(),
        ) else {
            return false;
        };
        (left, right) = (first, second);
    }
    false
}

fn delegates(ctx: &TsCtx<'_>, f: &Function<'_>, original: SymbolId) -> bool {
    if f.r#async || f.generator || !f.params.items.is_empty() {
        return false;
    }
    let Some(args) = f
        .params
        .rest
        .as_ref()
        .and_then(|r| r.rest.argument.get_binding_identifier())
        .and_then(|id| id.symbol_id.get())
    else {
        return false;
    };
    let Some((Statement::ReturnStatement(ret), before)) =
        f.body.as_ref().and_then(|b| b.statements.split_last())
    else {
        return false;
    };
    let arg_id = |e: &Expression<'_>| match resolve(ctx, e) {
        Expression::Identifier(id) => symbol(ctx, id) == Some(args),
        _ => false,
    };
    for st in before {
        let Statement::ExpressionStatement(st) = st else {
            return false;
        };
        let Expression::CallExpression(call) = &st.expression else {
            return false;
        };
        let inv = invoked(ctx, call);
        if inv
            .callee
            .get_member_expr()
            .is_some_and(|m| arg_id(m.object()))
            || (inv.mutation().is_some() && inv.arg(0).is_some_and(arg_id))
        {
            return false;
        }
    }
    let Some(Expression::CallExpression(c)) = &ret.argument else {
        return false;
    };
    c.arguments.len() == 3
        && path(ctx, &c.callee).is_some_and(|p| p == ["Reflect", "apply"])
        && matches!(c.arguments[0].as_expression(),Some(Expression::Identifier(id)) if symbol(ctx,id)==Some(original))
        && matches!(
            c.arguments[1].as_expression(),
            Some(Expression::ThisExpression(_))
        )
        && c.arguments[2].as_expression().is_some_and(arg_id)
}

fn target<'a>(a: &'a AssignmentExpression<'a>) -> Option<&'a MemberExpression<'a>> {
    a.left.as_member_expression()
}

pub(super) fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut saved = Vec::new();
    let mut assignments = Vec::new();
    for node in ctx.semantic.nodes().iter() {
        match node.kind() {
            AstKind::VariableDeclarator(v) => {
                if let Some(m) = v.init.as_ref().and_then(Expression::get_member_expr)
                    && let Some(id) =
                        v.id.get_binding_identifier()
                            .and_then(|b| b.symbol_id.get())
                    && !ctx.semantic.scoping().symbol_is_mutated(id)
                {
                    saved.push((v.span.start, id, m));
                }
            }
            AstKind::AssignmentExpression(a) if a.operator.is_assign() => {
                if let Some(m) = target(a) {
                    assignments.push((a, m));
                }
            }
            _ => (),
        }
    }
    let restores: Vec<_> = assignments
        .iter()
        .filter_map(|&(restore, member)| {
            let Expression::Identifier(id) = &restore.right else {
                return None;
            };
            Some((restore.span.start, symbol(ctx, id)?, member))
        })
        .collect();
    for &(a, m) in &assignments {
        let f = match resolve(ctx, &a.right) {
            Expression::FunctionExpression(f) => Some(&**f),
            Expression::Identifier(id) => match declaration(ctx, id) {
                Some(AstKind::Function(f)) => Some(f),
                _ => continue,
            },
            Expression::ArrowFunctionExpression(_) => None,
            _ => continue,
        };
        if writes(ctx, AstKind::AssignmentExpression(a))
            || super::bindings::assignment(ctx, AstKind::AssignmentExpression(a))
        {
            continue;
        }
        if MemberKeys::Resolve(ctx)
            .read(m)
            .is_some_and(|k| k.starts_with("on"))
            && path(ctx, m.object()).is_some_and(|p| {
                matches!(
                    p.as_slice(),
                    ["window" | "self" | "global" | "globalThis" | "document"]
                        | ["window" | "globalThis", "document"]
                )
            })
        {
            continue;
        }
        let has_later_assignment = |id| {
            restores.iter().any(|&(start, saved_id, member)| {
                start > a.span.start && saved_id == id && same(ctx, m, member)
            })
        };
        if saved.iter().any(|&(start, id, old)| {
            start < a.span.start
                && same(ctx, m, old)
                && !f.is_some_and(|f| delegates(ctx, f, id))
                && has_later_assignment(id)
        }) {
            findings.report(a.span.start as usize, None);
        }
    }
}
