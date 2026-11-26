use super::values::{
    MemberKeys, Origin, origin_path, outer, path_is, same_key, site_target, unwrapped,
};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{CallExpression, VariableDeclarationKind},
    },
    span::{GetSpan, Span},
    syntax::node::NodeId,
};
use std::collections::HashMap;

fn forwards_bind_argument(ctx: &TsCtx<'_>, call: &CallExpression<'_>, function: Span) -> bool {
    call.arguments
        .first()
        .and_then(|arg| arg.as_expression())
        .is_some_and(|arg| unwrapped(ctx, arg).span() == function)
}

fn immediate_call<'a>(ctx: &TsCtx<'a>, function: &oxc::semantic::AstNode<'a>) -> bool {
    let mut id = function.id();
    let callee = function.span();
    while let Some(parent) = outer(ctx, id) {
        match parent {
            AstKind::CallExpression(call) => {
                let Some((written, target)) = site_target(ctx, parent) else {
                    return false;
                };
                let in_callee = written.span().contains_inclusive(callee);
                if target.callee.span() == callee
                    && !target.function_apply
                    && (in_callee
                        || origin_path(ctx, written).is_some_and(|(origin, path)| {
                            origin == Origin::Global && path_is(ctx, &path, &["Reflect", "apply"])
                        }))
                {
                    return true;
                }
                if let Some(member) = target.callee.get_member_expr()
                    && MemberKeys(ctx)
                        .read(member)
                        .is_some_and(|key| same_key(ctx, key, "bind"))
                {
                    let receiver = in_callee && member.object().span().contains_inclusive(callee);
                    let forwarded = target.global_path().is_some_and(|path| {
                        path_is(ctx, path, &["Function", "prototype", "call", "bind"])
                    }) && forwards_bind_argument(ctx, call, callee);
                    if !receiver && !forwarded {
                        return false;
                    }
                } else if !in_callee {
                    return false;
                }
            }
            AstKind::SequenceExpression(_)
            | AstKind::StaticMemberExpression(_)
            | AstKind::ComputedMemberExpression(_) => {}
            _ => return false,
        }
        id = parent.node_id();
    }
    false
}

pub(super) fn suspension_starts(ctx: &TsCtx<'_>) -> Vec<u32> {
    let mut starts: Vec<_> = ctx
        .semantic
        .nodes()
        .iter()
        .take_while(|_| ctx.resolution.take_work(1))
        .filter(|node| match node.kind() {
            AstKind::AwaitExpression(_) | AstKind::YieldExpression(_) => true,
            AstKind::ForOfStatement(loop_) => loop_.r#await,
            AstKind::VariableDeclaration(declaration) => {
                declaration.kind == VariableDeclarationKind::AwaitUsing
            }
            _ => false,
        })
        .map(|node| node.span().start)
        .collect();
    if !starts.is_sorted() {
        let passes = starts.len().bit_width() as usize;
        if !ctx
            .resolution
            .take_work(starts.len().saturating_mul(passes))
        {
            return Vec::new();
        }
        starts.sort_unstable();
    }
    starts
}

pub(super) fn region(
    ctx: &TsCtx<'_>,
    regions: &mut HashMap<NodeId, Option<NodeId>>,
    mut id: NodeId,
) -> Option<NodeId> {
    let nodes = ctx.semantic.nodes();
    let mut path = Vec::new();
    let inherited = loop {
        if !ctx.resolution.take_work(1) {
            return None;
        }
        if let Some(&region) = regions.get(&id) {
            break region;
        }
        if id == NodeId::ROOT {
            break None;
        }
        path.push(id);
        let child = nodes.kind(id).span();
        let node = nodes.parent_node(id);
        let boundary = match node.kind() {
            AstKind::Decorator(_) => {
                let Some(class) = nodes
                    .ancestors(node.id())
                    .take_while(|_| ctx.resolution.take_work(1))
                    .find(|n| matches!(n.kind(), AstKind::Class(_)))
                else {
                    break None;
                };
                id = class.id();
                continue;
            }
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_) => {
                matches!(node.kind(), AstKind::Function(f) if f.generator)
                    || !immediate_call(ctx, node)
            }
            AstKind::PropertyDefinition(p) => {
                !p.r#static
                    && p.value
                        .as_ref()
                        .is_some_and(|value| value.span().contains_inclusive(child))
            }
            AstKind::AccessorProperty(p) => {
                !p.r#static
                    && p.value
                        .as_ref()
                        .is_some_and(|value| value.span().contains_inclusive(child))
            }
            _ => false,
        };
        if boundary {
            break Some(node.id());
        }
        id = node.id();
    };
    regions.extend(path.into_iter().map(|id| (id, inherited)));
    inherited
}
