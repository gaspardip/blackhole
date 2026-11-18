use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, IdentifierReference, VariableDeclarationKind},
    },
    syntax::{node::NodeId, symbol::SymbolId},
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

pub(crate) fn symbol(ctx: &TsCtx<'_>, ident: &IdentifierReference<'_>) -> Option<SymbolId> {
    ctx.semantic
        .scoping()
        .get_reference(ident.reference_id.get()?)
        .symbol_id()
}
pub(crate) fn declaration(ctx: &TsCtx<'_>, ident: &IdentifierReference<'_>) -> Option<NodeId> {
    Some(
        ctx.semantic
            .scoping()
            .symbol_declaration(symbol(ctx, ident)?),
    )
}

#[derive(Default)]
pub(crate) struct ConstBindings(HashMap<SymbolId, bool>);
impl ConstBindings {
    pub(crate) fn immutable(&mut self, ctx: &TsCtx<'_>, symbol: SymbolId, id: NodeId) -> bool {
        *self.0.entry(symbol).or_insert_with(|| {
            let AstKind::VariableDeclaration(var) = ctx.semantic.nodes().parent_kind(id) else {
                return false;
            };
            var.kind == VariableDeclarationKind::Const
                && ctx
                    .semantic
                    .scoping()
                    .get_resolved_references(symbol)
                    .all(|r| !r.is_write())
        })
    }
    pub(crate) fn walk<'a, T>(
        &mut self,
        ctx: &TsCtx<'a>,
        mut expr: &'a Expression<'a>,
        mut seen: HashSet<NodeId>,
        inspect: impl Fn(&'a Expression<'a>) -> ControlFlow<Option<T>, &'a IdentifierReference<'a>>,
        binding: impl Fn(NodeId, AstKind<'a>) -> ControlFlow<Option<T>>,
    ) -> Option<T> {
        loop {
            let ident = match inspect(expr) {
                ControlFlow::Break(end) => return end,
                ControlFlow::Continue(ident) => ident,
            };
            let symbol = symbol(ctx, ident)?;
            let id = ctx.semantic.scoping().symbol_declaration(symbol);
            if !seen.insert(id) {
                return None;
            }
            let kind = ctx.semantic.nodes().kind(id);
            if let ControlFlow::Break(end) = binding(id, kind) {
                return end;
            }
            let AstKind::VariableDeclarator(var) = kind else {
                return None;
            };
            if !self.immutable(ctx, symbol, id) {
                return None;
            }
            expr = var.init.as_ref()?;
        }
    }
}
