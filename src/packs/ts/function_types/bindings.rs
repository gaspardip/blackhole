use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, IdentifierReference, VariableDeclarationKind},
    },
    syntax::{node::NodeId, symbol::SymbolId},
};
use std::{collections::HashMap, ops::ControlFlow};

pub(crate) fn symbol(ctx: &TsCtx<'_>, ident: &IdentifierReference<'_>) -> Option<SymbolId> {
    ctx.semantic
        .scoping()
        .get_reference(ident.reference_id.get()?)
        .symbol_id()
}
pub(crate) fn declaration(ctx: &TsCtx<'_>, ident: &IdentifierReference<'_>) -> Option<NodeId> {
    value_declarations(ctx, symbol(ctx, ident)?).next()
}

pub(crate) fn is_type_declaration(kind: AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::TSTypeAliasDeclaration(_) | AstKind::TSInterfaceDeclaration(_)
    )
}

pub(crate) fn value_declarations<'a>(
    ctx: &'a TsCtx<'_>,
    symbol: SymbolId,
) -> impl Iterator<Item = NodeId> + 'a {
    ctx.semantic
        .scoping()
        .symbol_declarations(symbol)
        .filter(|&id| {
            let nodes = ctx.semantic.nodes();
            match nodes.kind(id) {
                AstKind::ImportSpecifier(s) if s.import_kind.is_type() => false,
                AstKind::ImportSpecifier(_) | AstKind::ImportNamespaceSpecifier(_) | AstKind::ImportDefaultSpecifier(_) =>
                    !matches!(nodes.parent_kind(id), AstKind::ImportDeclaration(i) if i.import_kind.is_type()),
                kind => !is_type_declaration(kind),
            }
        })
}

pub(crate) struct ConstBindings<T> {
    immutable: HashMap<SymbolId, bool>,
    origins: HashMap<(Option<NodeId>, NodeId), Option<T>>,
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
                    .semantic
                    .scoping()
                    .get_resolved_references(symbol)
                    .all(|r| !r.is_write())
        })
    }
    pub(crate) fn walk<'a>(
        &mut self,
        ctx: &TsCtx<'a>,
        mut expr: &'a Expression<'a>,
        owner: Option<NodeId>,
        inspect: impl Fn(&'a Expression<'a>) -> ControlFlow<Option<T>, &'a IdentifierReference<'a>>,
        binding: impl Fn(NodeId, AstKind<'a>) -> ControlFlow<Option<T>>,
    ) -> Option<T> {
        let mut path = Vec::new();
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
            if let Some(end) = self.origins.get(&key) {
                break *end;
            }
            self.origins.insert(key, None);
            path.push(key);
            let kind = ctx.semantic.nodes().kind(id);
            if let ControlFlow::Break(end) = binding(id, kind) {
                break end;
            }
            let AstKind::VariableDeclarator(var) = kind else {
                break None;
            };
            if !self.immutable(ctx, sym, id) {
                break None;
            }
            let Some(init) = &var.init else {
                break None;
            };
            expr = init;
        };
        for key in path {
            self.origins.insert(key, end);
        }
        end
    }
}
