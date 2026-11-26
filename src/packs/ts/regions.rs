use super::values::{
    MemberKeys, Origin, inner, origin_path, outer, path_is, same_key, site_target, unwrapped,
};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Argument, CallExpression, Expression, VariableDeclarationKind},
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

fn immediate_call<'a>(
    ctx: &TsCtx<'a>,
    regions: &Regions,
    function: &oxc::semantic::AstNode<'a>,
) -> bool {
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
                        || (call
                            .arguments
                            .get(2)
                            .and_then(Argument::as_expression)
                            .is_some_and(|args| {
                                matches!(inner(ctx, args), Expression::ArrayExpression(_))
                            })
                            && origin_path(ctx, written).is_some_and(|(origin, path)| {
                                origin == Origin::Global
                                    && path_is(ctx, &path, &["Reflect", "apply"])
                            })))
                {
                    return regions.uninterrupted(ctx, callee.end, call.span.end);
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

fn suspension_starts(ctx: &TsCtx<'_>) -> Vec<u32> {
    let mut starts: Vec<_> = ctx
        .bounded_nodes()
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
        if !ctx.take_work(starts.len().saturating_mul(passes)) {
            return Vec::new();
        }
        starts.sort_unstable();
    }
    starts
}

pub(super) struct Regions {
    boundaries: HashMap<NodeId, Option<NodeId>>,
    suspensions: Vec<u32>,
}

impl Regions {
    pub(super) fn new(ctx: &TsCtx<'_>) -> Self {
        Self {
            boundaries: HashMap::new(),
            suspensions: suspension_starts(ctx),
        }
    }

    pub(super) fn uninterrupted(&self, ctx: &TsCtx<'_>, start: u32, end: u32) -> bool {
        ctx.take_work(self.suspensions.len().bit_width() as usize)
            && self
                .suspensions
                .get(self.suspensions.partition_point(|&at| at < start))
                .is_none_or(|&at| at >= end)
    }
}

pub(super) fn region(ctx: &TsCtx<'_>, regions: &mut Regions, mut id: NodeId) -> Option<NodeId> {
    let nodes = ctx.semantic.nodes();
    let mut path = Vec::new();
    let inherited = loop {
        if !ctx.take_work(1) {
            return None;
        }
        if let Some(&region) = regions.boundaries.get(&id) {
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
                    .take_while(|_| ctx.take_work(1))
                    .find(|n| matches!(n.kind(), AstKind::Class(_)))
                else {
                    break None;
                };
                id = class.id();
                continue;
            }
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_) => {
                matches!(node.kind(), AstKind::Function(f) if f.generator)
                    || !immediate_call(ctx, regions, node)
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
    regions
        .boundaries
        .extend(path.into_iter().map(|id| (id, inherited)));
    inherited
}
