use super::{
    double_mutations::Mutation,
    values::{Invocation, MemberKeys, declaration, enclosing_call, path, resolve},
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

pub(super) fn contains_function<'a>(ctx: &TsCtx<'a>, initial: &'a Expression<'a>) -> bool {
    let mut pending = vec![initial];
    let mut seen = HashSet::new();
    while let Some(next) = pending.pop() {
        let e = resolve(ctx, next);
        if !seen.insert(e.span()) {
            continue;
        }
        if let Some(c) = super::values::call_expr(e) {
            if let Some(m) = resolve(ctx, &c.callee).get_member_expr()
                && MemberKeys(ctx).read(m) == Some("bind")
                && !matches!(resolve(ctx, m.object()), Expression::ObjectExpression(_))
            {
                pending.push(m.object());
            }

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
fn binding<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> bool {
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
pub(super) fn mutation<'a>(ctx: &TsCtx<'a>, call: &Invocation<'a>) -> bool {
    if call.path.as_deref() == Some(&["cloudflare:workers", "withEnv"]) {
        return call.arg(0).is_some_and(|arg| contains_function(ctx, arg));
    }
    let Some(kind) = call.mutation() else {
        return false;
    };
    if !call.arg(0).is_some_and(|e| binding(ctx, e)) {
        return false;
    }
    match kind {
        Mutation::Assign => call
            .args
            .iter()
            .skip(1)
            .flatten()
            .any(|e| contains_function(ctx, e)),
        Mutation::SetProperty => call.arg(2).is_some_and(|e| contains_function(ctx, e)),
        Mutation::DefineMany => call.arg(1).is_some_and(|e| contains_function(ctx, e)),
        Mutation::Delete => false,
    }
}
pub(super) fn assignment<'a>(ctx: &TsCtx<'a>, node: AstKind<'a>) -> bool {
    let AstKind::AssignmentExpression(a) = node else {
        return false;
    };
    a.left
        .as_member_expression()
        .is_some_and(|m| binding(ctx, m.object()))
        && contains_function(ctx, &a.right)
}
pub(super) fn spread<'a>(ctx: &TsCtx<'a>, obj: &'a ObjectExpression<'a>) -> bool {
    let has_binding_spread = obj.properties.iter().any(|property| match property {
        ObjectPropertyKind::SpreadProperty(spread) => binding(ctx, &spread.argument),
        ObjectPropertyKind::ObjectProperty(_) => false,
    });
    if !has_binding_spread {
        return false;
    }
    if enclosing_call(ctx, obj.node_id.get())
        .is_some_and(|call| call.path.as_deref() == Some(&["cloudflare:workers", "withEnv"]))
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
