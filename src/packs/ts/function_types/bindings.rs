pub(in crate::packs::ts) use super::super::values::symbol;
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, IdentifierReference, VariableDeclarationKind},
    },
    syntax::{
        node::NodeId,
        symbol::{SymbolFlags, SymbolId},
    },
};
use std::{collections::HashMap, ops::ControlFlow};

pub(crate) fn declaration(ctx: &TsCtx<'_>, ident: &IdentifierReference<'_>) -> Option<NodeId> {
    value_declarations(ctx, symbol(ctx, ident)?).next()
}

pub(crate) fn declarations<'a>(
    ctx: &'a TsCtx<'_>,
    symbol: SymbolId,
) -> impl Iterator<Item = (NodeId, SymbolFlags)> + 'a {
    let scopes = ctx.semantic.scoping();
    let merged = scopes.symbol_redeclarations(symbol);
    let first = merged.is_empty().then(|| {
        (
            scopes.symbol_declaration(symbol),
            scopes.symbol_flags(symbol),
        )
    });
    first.into_iter()
        .chain(ctx.counted(merged.iter()).map(|d| (d.declaration, d.flags)))
        .map(|(id, flags)| {
            // The binder marks import-equals as a value import even for the type-only spelling.
            let type_only = matches!(ctx.semantic.nodes().kind(id), AstKind::TSImportEqualsDeclaration(i) if i.import_kind.is_type());
            if flags.contains(SymbolFlags::Import) && type_only {
                (id, SymbolFlags::TypeImport)
            } else { (id, flags) }
        })
}

pub(crate) fn type_declarations<'a>(
    ctx: &'a TsCtx<'_>,
    symbol: SymbolId,
) -> impl Iterator<Item = NodeId> + 'a {
    declarations(ctx, symbol).filter_map(|(id, flags)| {
        flags
            .intersects(SymbolFlags::Type | SymbolFlags::Import | SymbolFlags::TypeImport)
            .then_some(id)
    })
}

pub(crate) fn value_declarations<'a>(
    ctx: &'a TsCtx<'_>,
    symbol: SymbolId,
) -> impl Iterator<Item = NodeId> + 'a {
    declarations(ctx, symbol).filter_map(|(id, flags)| flags.is_value().then_some(id))
}

#[derive(Clone, Copy)]
enum Origin<T> {
    Finished(Option<T>),
    RetryAt(NodeId),
}

pub(crate) struct ConstBindings<T> {
    immutable: HashMap<SymbolId, bool>,
    origins: HashMap<(Option<NodeId>, NodeId), Origin<T>>,
}
impl<T> Default for ConstBindings<T> {
    fn default() -> Self {
        Self {
            immutable: HashMap::new(),
            origins: HashMap::new(),
        }
    }
}
impl<T: Copy> ConstBindings<T> {
    pub(crate) fn immutable(&mut self, ctx: &TsCtx<'_>, symbol: SymbolId, id: NodeId) -> bool {
        *self.immutable.entry(symbol).or_insert_with(|| {
            let AstKind::VariableDeclaration(var) = ctx.semantic.nodes().parent_kind(id) else {
                return false;
            };
            var.kind == VariableDeclarationKind::Const
                && ctx
                    .counted(ctx.semantic.scoping().get_resolved_references(symbol))
                    .all(|r| !r.is_write())
        })
    }
    pub(crate) fn walk<'a>(
        &mut self,
        ctx: &TsCtx<'a>,
        mut expr: &'a Expression<'a>,
        owner: Option<NodeId>,
        finished: impl Fn(T) -> bool,
        inspect: impl Fn(&'a Expression<'a>) -> ControlFlow<Option<T>, &'a IdentifierReference<'a>>,
        binding: impl Fn(NodeId, AstKind<'a>) -> ControlFlow<Option<T>>,
    ) -> Option<T> {
        let mut path = std::collections::HashSet::new();
        let mut last_binding = None;
        let end = loop {
            let ident = match inspect(expr) {
                ControlFlow::Break(end) => break end,
                ControlFlow::Continue(i) => i,
            };
            let Some(sym) = symbol(ctx, ident) else {
                break None;
            };
            let Some(id) = value_declarations(ctx, sym).next() else {
                break None;
            };
            let key = (owner, id);
            let id = match self.origins.get(&key) {
                Some(Origin::Finished(end)) => break *end,
                Some(Origin::RetryAt(id)) => *id,
                None => id,
            };
            last_binding = Some(id);
            if !path.insert(key) {
                break None;
            }
            let kind = ctx.semantic.nodes().kind(id);
            if let ControlFlow::Break(end) = binding(id, kind) {
                break end;
            }
            let Some(init) = super::direct_binding(kind).and_then(|b| b.initializer) else {
                break None;
            };
            if !self.immutable(ctx, sym, id) {
                break None;
            }
            ctx.record_work(1);
            expr = init;
        };
        let origin = match (end, last_binding) {
            (Some(value), Some(id)) if !finished(value) => Origin::RetryAt(id),
            _ => Origin::Finished(end),
        };
        for key in path {
            self.origins.insert(key, origin);
        }
        end
    }
}
