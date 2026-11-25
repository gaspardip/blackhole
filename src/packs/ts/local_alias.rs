use super::values::symbol;
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, ModuleExportName},
    },
    syntax::symbol::SymbolId,
};

use std::collections::HashSet;

#[derive(Default)]
pub(super) struct NamedExports {
    values: HashSet<SymbolId>,
    types: HashSet<SymbolId>,
}

fn named_exports<'ctx>(ctx: &'ctx TsCtx<'_>) -> &'ctx NamedExports {
    ctx.resolution.named_exports.get_or_init(|| {
        let mut exports = NamedExports::default();
        for node in ctx.semantic.nodes().iter() {
            let value = match node.kind() {
                AstKind::ExportNamedDeclaration(decl) => {
                    for spec in &decl.specifiers {
                        if let ModuleExportName::IdentifierReference(local) = &spec.local
                            && let Some(symbol) = ctx
                                .semantic
                                .scoping()
                                .find_binding(node.scope_id(), local.name)
                        {
                            if decl.export_kind.is_type() || spec.export_kind.is_type() {
                                exports.types.insert(symbol);
                            } else {
                                exports.values.insert(symbol);
                            }
                        }
                    }
                    continue;
                }
                AstKind::ExportDefaultDeclaration(decl) => decl.declaration.as_expression(),
                AstKind::TSExportAssignment(decl) => Some(&decl.expression),
                _ => continue,
            };
            if let Some(Expression::Identifier(id)) = value.map(Expression::get_inner_expression)
                && let Some(binding) = symbol(ctx, id)
            {
                exports.values.insert(binding);
            }
        }
        exports
    })
}

pub(super) fn is_spread_alias(ctx: &TsCtx<'_>, symbol: SymbolId) -> bool {
    // A local type export can keep an empty spread alias observable.
    is_local(ctx, symbol) && !named_exports(ctx).types.contains(&symbol)
}

pub(super) fn is_local(ctx: &TsCtx<'_>, symbol: SymbolId) -> bool {
    ctx.semantic
        .scoping()
        .symbol_flags(symbol)
        .is_const_variable()
        && !is_exported(ctx, symbol)
}

fn is_exported(ctx: &TsCtx<'_>, symbol: SymbolId) -> bool {
    let declaration = ctx.semantic.symbol_declaration(symbol);
    ctx.semantic
        .nodes()
        .ancestor_kinds(declaration.id())
        .take(2)
        .any(|kind| {
            matches!(
                kind,
                AstKind::ExportNamedDeclaration(_)
                    | AstKind::ExportDefaultDeclaration(_)
                    | AstKind::ExportDeclaration(_)
            )
        })
        || named_exports(ctx).values.contains(&symbol)
}
