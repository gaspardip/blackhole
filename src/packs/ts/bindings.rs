use super::{
    double_mutations::{Invocation, Mutation, invoked},
    values::{declaration, member_key, path, resolve},
};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{BindingPattern, Expression, ObjectExpression, ObjectPropertyKind},
    },
    span::GetSpan,
};
use std::collections::HashSet;

pub(super) fn contains_function(ctx: &TsCtx<'_>, initial: &Expression<'_>) -> bool {
    let mut pending = vec![initial];
    let mut seen = HashSet::new();
    while let Some(next) = pending.pop() {
        let e = resolve(ctx, next);
        if !seen.insert(e.span()) {
            continue;
        }
        match e {
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
                return true;
            }
            Expression::Identifier(id)
                if matches!(declaration(ctx, id), Some(AstKind::Function(_))) =>
            {
                return true;
            }
            Expression::CallExpression(c) => {
                if let Some(m) = resolve(ctx, &c.callee).get_member_expr()
                    && member_key(ctx, m) == Some("bind")
                    && !matches!(resolve(ctx, m.object()), Expression::ObjectExpression(_))
                {
                    pending.push(m.object());
                }
            }
            Expression::ObjectExpression(obj) => {
                for p in &obj.properties {
                    pending.push(match p {
                        ObjectPropertyKind::SpreadProperty(s) => &s.argument,
                        ObjectPropertyKind::ObjectProperty(o) => &o.value,
                    });
                }
            }
            _ => pending.extend(super::values::alternatives(e)),
        }
    }
    false
}
fn binding(ctx: &TsCtx<'_>, expr: &Expression<'_>) -> bool {
    if path(ctx, expr).is_some_and(|p| p.starts_with(&["cloudflare:workers", "env"])) {
        return true;
    }
    let mut e = expr;
    let mut seen = HashSet::new();
    loop {
        e = resolve(ctx, e);
        if !seen.insert(e.span()) {
            return false;
        }
        if let Some(m) = e.get_member_expr() {
            e = m.object();
            continue;
        }
        let Expression::Identifier(id) = e else {
            return false;
        };
        if let Some(AstKind::VariableDeclarator(v)) = declaration(ctx, id)
            && matches!(v.id, BindingPattern::ObjectPattern(_))
            && let Some(init) = &v.init
        {
            e = init;
            continue;
        }
        let Some(symbol) = super::values::symbol(ctx, id) else {
            return false;
        };
        return super::values::callback_call(ctx, symbol, 2).is_some_and(|call| {
            call.path.as_deref() == Some(&["cloudflare:test", "runInDurableObject"])
        });
    }
}
pub(super) fn mutation(ctx: &TsCtx<'_>, call: &Invocation<'_>) -> bool {
    if call.path.as_deref() == Some(&["cloudflare:workers", "withEnv"]) {
        return call.arg(0).is_some_and(|arg| contains_function(ctx, arg));
    }
    if !call.arg(0).is_some_and(|e| binding(ctx, e)) {
        return false;
    }
    match call.mutation() {
        Some(Mutation::Assign) => call
            .args
            .iter()
            .skip(1)
            .flatten()
            .any(|e| contains_function(ctx, e)),
        Some(Mutation::SetProperty) => call.arg(2).is_some_and(|e| contains_function(ctx, e)),
        Some(Mutation::DefineMany) => call.arg(1).is_some_and(|e| contains_function(ctx, e)),
        _ => false,
    }
}
pub(super) fn assignment(ctx: &TsCtx<'_>, node: AstKind<'_>) -> bool {
    let AstKind::AssignmentExpression(a) = node else {
        return false;
    };
    a.left
        .as_simple_assignment_target()
        .and_then(|t| t.as_member_expression())
        .is_some_and(|m| binding(ctx, m.object()))
        && contains_function(ctx, &a.right)
}
pub(super) fn spread(ctx: &TsCtx<'_>, obj: &ObjectExpression<'_>) -> bool {
    let has_binding_spread = obj.properties.iter().any(|property| match property {
        ObjectPropertyKind::SpreadProperty(spread) => binding(ctx, &spread.argument),
        ObjectPropertyKind::ObjectProperty(_) => false,
    });
    if !has_binding_spread {
        return false;
    }
    if let AstKind::CallExpression(c) = ctx.semantic.nodes().parent_kind(obj.node_id.get())
        && invoked(ctx, c).path.as_deref() == Some(&["cloudflare:workers", "withEnv"])
    {
        return false;
    }
    obj.properties.iter().any(|p| {
        contains_function(
            ctx,
            match p {
                ObjectPropertyKind::SpreadProperty(s) => &s.argument,
                ObjectPropertyKind::ObjectProperty(o) => &o.value,
            },
        )
    })
}
