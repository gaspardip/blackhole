use super::{
    double_mutations::{Invocation, invoked},
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
                if let Some(m) = resolve(ctx, &c.callee).as_member_expression()
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
            Expression::ConditionalExpression(c) => pending.extend([&c.consequent, &c.alternate]),
            Expression::LogicalExpression(l) => pending.extend([&l.left, &l.right]),
            Expression::SequenceExpression(s) => pending.extend(s.expressions.last()),
            _ => (),
        }
    }
    false
}
fn binding(ctx: &TsCtx<'_>, expr: &Expression<'_>) -> bool {
    if path(ctx, expr).is_some_and(|p| p.starts_with(&["cloudflare:workers".into(), "env".into()]))
    {
        return true;
    }
    let mut e = expr;
    let mut seen = HashSet::new();
    loop {
        e = resolve(ctx, e);
        if !seen.insert(e.span()) {
            return false;
        }
        if let Some(m) = e.as_member_expression() {
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
        let Some(symbol) = id
            .reference_id
            .get()
            .and_then(|r| ctx.semantic.scoping().get_reference(r).symbol_id())
        else {
            return false;
        };
        let decl = ctx.semantic.symbol_declaration(symbol);
        let nodes = ctx.semantic.nodes();
        let Some(function) = nodes.ancestors(decl.id()).find(|n| {
            matches!(
                n.kind(),
                AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
            )
        }) else {
            return false;
        };
        let params = match function.kind() {
            AstKind::Function(f) => &f.params,
            AstKind::ArrowFunctionExpression(f) => &f.params,
            _ => return false,
        };
        if !params.items.iter().take(2).any(|p| {
            p.pattern
                .get_binding_identifier()
                .is_some_and(|b| b.symbol_id.get() == Some(symbol))
        }) {
            return false;
        }
        let AstKind::CallExpression(c) = nodes.parent_kind(function.id()) else {
            return false;
        };
        let call = invoked(ctx, c);
        return call.arg(1).is_some_and(|e| e.span() == function.span())
            && path(ctx, call.callee)
                .is_some_and(|p| p == ["cloudflare:test", "runInDurableObject"]);
    }
}
pub(super) fn mutation(ctx: &TsCtx<'_>, call: &Invocation<'_>) -> bool {
    let Some(p) = path(ctx, call.callee) else {
        return false;
    };
    if p == ["cloudflare:workers", "withEnv"] {
        return call.arg(0).is_some_and(|arg| contains_function(ctx, arg));
    }
    if !call.arg(0).is_some_and(|e| binding(ctx, e)) {
        return false;
    }
    let p = p.join(".");
    match p.as_str() {
        "Object.assign" => call
            .args
            .iter()
            .skip(1)
            .flatten()
            .any(|e| contains_function(ctx, e)),
        "Object.defineProperty" | "Reflect.defineProperty" | "Reflect.set" => {
            call.arg(2).is_some_and(|e| contains_function(ctx, e))
        }
        "Object.defineProperties" => call.arg(1).is_some_and(|e| contains_function(ctx, e)),
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
    if let AstKind::CallExpression(c) = ctx.semantic.nodes().parent_kind(obj.node_id.get())
        && path(ctx, invoked(ctx, c).callee).is_some_and(|p| p == ["cloudflare:workers", "withEnv"])
    {
        return false;
    }
    obj.properties
        .iter()
        .any(|p| matches!(p,ObjectPropertyKind::SpreadProperty(s) if binding(ctx,&s.argument)))
        && obj.properties.iter().any(|p| {
            contains_function(
                ctx,
                match p {
                    ObjectPropertyKind::SpreadProperty(s) => &s.argument,
                    ObjectPropertyKind::ObjectProperty(o) => &o.value,
                },
            )
        })
}
