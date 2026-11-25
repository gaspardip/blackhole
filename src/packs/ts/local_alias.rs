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

fn named_exports(ctx: &TsCtx<'_>) -> HashSet<SymbolId> {
    let mut exported = HashSet::new();
    for node in ctx.semantic.nodes().iter() {
        let value = match node.kind() {
            AstKind::ExportNamedDeclaration(decl) if !decl.export_kind.is_type() => {
                for spec in &decl.specifiers {
                    if !spec.export_kind.is_type()
                        && let ModuleExportName::IdentifierReference(local) = &spec.local
                        && let Some(symbol) = ctx
                            .semantic
                            .scoping()
                            .find_binding(node.scope_id(), local.name)
                    {
                        exported.insert(symbol);
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
            exported.insert(binding);
        }
    }
    exported
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
        || ctx
            .resolution
            .named_exports
            .get_or_init(|| named_exports(ctx))
            .contains(&symbol)
}
