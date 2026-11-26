use super::module_object::{Kinds, LoaderBase, Value};
use crate::engine::TsCtx;
use crate::packs::ts::values::{inner, symbol};
use oxc::{
    ast::{
        AstKind,
        ast::{
            CallExpression, Expression, ImportDeclarationSpecifier, ModuleExportName,
            ObjectExpression, ObjectPropertyKind, Statement, StringLiteral, WithClause,
        },
    },
    span::GetSpan,
    syntax::node::NodeId,
};
use oxc_resolver::PackageType;
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Static,
    StaticRequire,
    TypeRequire,
    TypeImport,
    TypeDeclaration,
    Dynamic,
    Require,
}
impl Kind {
    pub(super) fn commonjs_call(self) -> bool {
        self == Self::Require
    }
    pub(super) fn implicit_mode(self) -> bool {
        matches!(self, Self::Static | Self::TypeDeclaration)
    }
    fn declaration(ctx: &TsCtx<'_>, type_only: bool, attrs: Option<&WithClause<'_>>) -> Self {
        if !type_only {
            return Self::Static;
        }
        let mode = attrs
            .and_then(|attrs| {
                ctx.counted(attrs.with_entries.iter())
                    .find(|a| a.key.as_js_str().as_str() == Some("resolution-mode"))
            })
            .and_then(|a| a.value.value.as_str());
        Self::type_mode(mode)
    }
    fn for_importer(self, in_declaration: bool) -> Self {
        match (in_declaration, self) {
            (true, Self::Static) => Self::TypeDeclaration,
            (true, Self::StaticRequire) => Self::TypeRequire,
            (_, kind) => kind,
        }
    }
    fn require_declaration(type_only: bool) -> Self {
        if type_only {
            Self::TypeRequire
        } else {
            Self::StaticRequire
        }
    }
    fn type_mode(mode: Option<&str>) -> Self {
        match mode {
            Some("require") => Self::TypeRequire,
            Some("import") => Self::TypeImport,
            _ => Self::TypeDeclaration,
        }
    }
}

pub(super) struct Import {
    pub offset: u32,
    pub name: Option<String>,
    pub kind: Kind,
    pub filename_base: Option<PackageType>,
    pub verbatim_only: bool,
}

impl Import {
    pub(super) fn needs_format(&self, typescript: bool) -> bool {
        self.filename_base.is_some()
            || (typescript
                && self.kind.implicit_mode()
                && self.name.as_deref().is_some_and(super::specifier::bare))
    }
    fn new(offset: u32, name: Option<&str>, kind: Kind) -> Self {
        Self {
            offset,
            name: name.map(str::to_owned),
            kind,
            filename_base: None,
            verbatim_only: false,
        }
    }
    fn argument<'a>(kinds: &Kinds<'_, 'a>, expr: &'a Expression<'a>, kind: Kind) -> Self {
        let name = kinds.values.literal(expr);
        let at = if name.is_some() {
            inner(kinds.ctx, expr).span().start
        } else {
            expr.span().start
        };
        Self::new(at, name, kind)
    }
    fn literal(value: &StringLiteral<'_>, kind: Kind) -> Self {
        Self::new(value.span.start, value.value.as_str(), kind)
    }
}

fn type_import(specifier: &ImportDeclarationSpecifier<'_>) -> bool {
    matches!(specifier,ImportDeclarationSpecifier::ImportSpecifier(i) if i.import_kind.is_type())
}

fn type_exports(ctx: &TsCtx<'_>) -> HashSet<NodeId> {
    let scoping = ctx.semantic.scoping();
    let mut exports = HashSet::new();
    for stmt in ctx.counted(ctx.program.body.iter()) {
        let Statement::ExportNamedDeclaration(d) = stmt else {
            continue;
        };
        for specifier in ctx.counted(d.specifiers.iter()) {
            if (d.export_kind.is_type() || specifier.export_kind.is_type())
                && let ModuleExportName::IdentifierReference(id) = &specifier.local
                && let Some(binding) = symbol(ctx, id)
            {
                exports.insert(scoping.symbol_declaration(binding));
            }
        }
    }
    exports
}

fn declaration_readings(
    source: &StringLiteral<'_>,
    kind: impl Fn(bool) -> Kind,
    type_only: bool,
    can_emit_verbatim: bool,
    bindings: impl Iterator<Item = bool>,
) -> impl Iterator<Item = Import> {
    let mut present = [false; 2];
    for types in bindings {
        present[usize::from(type_only || types)] = true;
    }
    if present == [false, false] {
        present[usize::from(type_only)] = true;
    }
    [false, true]
        .into_iter()
        .zip(present)
        .filter(|&(_, present)| present)
        .map(move |(types, _)| Import::literal(source, kind(types)))
        .chain(
            (can_emit_verbatim && !type_only && !present[0]).then(|| Import {
                verbatim_only: true,
                ..Import::literal(source, Kind::Static)
            }),
        )
}

fn call_use<'a>(kinds: &Kinds<'_, 'a>, c: &'a CallExpression<'a>) -> Option<Import> {
    let value = kinds.kind_of(&c.callee)?;
    let arg = c.arguments.first();
    let expr = arg.and_then(oxc::ast::ast::Argument::as_expression);
    let unknown = || {
        Import::new(
            arg.map_or(c.span.start, |a| a.span().start),
            None,
            Kind::Require,
        )
    };
    match value {
        Value::Loader(LoaderBase::Unknown) => Some(unknown()),
        Value::Builtin => {
            Some(expr.map_or_else(unknown, |e| Import::argument(kinds, e, Kind::Require)))
        }
        Value::Loader(base) => {
            let mut specifier =
                expr.map_or_else(unknown, |e| Import::argument(kinds, e, Kind::Require));
            if specifier.name.is_some() {
                specifier.filename_base = base.filename_format();
            }
            Some(specifier)
        }
        _ => None,
    }
}

fn option<'a>(
    ctx: &TsCtx<'_>,
    object: &'a ObjectExpression<'a>,
    key: &str,
) -> Option<&'a Expression<'a>> {
    ctx.counted(object.properties.iter())
        .find_map(|prop| match prop {
            ObjectPropertyKind::ObjectProperty(p)
                if p.key.static_name().is_some_and(|name| name == key) =>
            {
                Some(&p.value)
            }
            _ => None,
        })
}
fn type_options(ctx: &TsCtx<'_>, object: Option<&ObjectExpression<'_>>) -> Kind {
    let mode = object
        .and_then(|obj| option(ctx, obj, "with"))
        .and_then(|value| match value {
            Expression::ObjectExpression(attrs) => option(ctx, attrs, "resolution-mode"),
            _ => None,
        })
        .and_then(|value| match value {
            Expression::StringLiteral(s) => s.value.as_str(),
            _ => None,
        });
    Kind::type_mode(mode)
}

fn builtin_value(source: &StringLiteral<'_>) -> Option<Value> {
    super::BuiltinSource::source(source.value.as_str()).map(super::BuiltinSource::value)
}

fn unread_binding<'a>(kinds: &Kinds<'_, 'a>, node: AstKind<'a>) -> Option<u32> {
    let nodes = kinds.ctx.semantic.nodes();
    match node {
        AstKind::ImportSpecifier(s) if !s.import_kind.is_type() => {
            let AstKind::ImportDeclaration(d) = nodes.parent_kind(node.node_id()) else {
                return None;
            };
            (!d.import_kind.is_type()
                && builtin_value(&d.source)?.member(Some(s.imported.name().as_str()))
                    == Some(Value::UnreadMember))
            .then_some(s.imported.span().start)
        }
        AstKind::ExportFromDeclaration(d) if !d.export_kind.is_type() => {
            let root = builtin_value(&d.source)?;
            kinds
                .ctx
                .counted(d.specifiers.iter())
                .any(|s| {
                    !s.export_kind.is_type()
                        && root
                            .member(Some(s.local.name().as_str()))
                            .is_some_and(Value::escapes)
                })
                .then_some(d.source.span.start)
        }
        AstKind::ExportAllDeclaration(d) if !d.export_kind.is_type() => {
            let root = builtin_value(&d.source)?;
            (d.exported.is_none() || root.escapes()).then_some(d.source.span.start)
        }
        AstKind::TSImportEqualsDeclaration(d)
            if !d.import_kind.is_type()
                && matches!(
                    nodes.parent_kind(node.node_id()),
                    AstKind::ExportDeclaration(_)
                ) =>
        {
            let oxc::ast::ast::TSModuleReference::ExternalModuleReference(r) = &d.module_reference
            else {
                return None;
            };
            builtin_value(&r.expression)?
                .escapes()
                .then_some(r.expression.span.start)
        }
        AstKind::JSXOpeningElement(e) => {
            let oxc::ast::ast::JSXElementName::MemberExpression(m) = &e.name else {
                return None;
            };
            kinds.jsx_member(m)?.escapes().then_some(m.span.start)
        }
        _ => None,
    }
}

pub(super) fn collect(ctx: &TsCtx<'_>, in_declaration: bool) -> Vec<Import> {
    let mut imports = Vec::new();
    let kinds = Kinds::new(ctx);
    let exported = type_exports(ctx);
    let can_emit_verbatim = ctx.program.source_type.is_typescript() && !in_declaration;
    for (id, node) in ctx.nodes_with_ids() {
        let kind = node.kind();
        let start = kind.span().start;
        let offsets = [
            super::bases::base_write(&kinds, id, kind).then_some(start),
            kinds
                .node(id)
                .is_some_and(|value| !kinds.read(id, value))
                .then_some(start),
            kinds.unread_alias(kind).map(|span| span.start),
            unread_binding(&kinds, kind),
        ];
        for offset in offsets.into_iter().flatten() {
            imports.push(Import::new(offset, None, Kind::Require));
        }
        let import = match kind {
            AstKind::ImportDeclaration(d) => {
                let bindings = ctx.counted(d.specifiers.iter().flatten()).flat_map(|s| {
                    let type_only = type_import(s);
                    [
                        Some(type_only),
                        exported.contains(&s.node_id()).then_some(true),
                    ]
                    .into_iter()
                    .flatten()
                });
                imports.extend(declaration_readings(
                    &d.source,
                    |types| Kind::declaration(ctx, types, d.with_clause.as_deref()),
                    d.import_kind.is_type(),
                    can_emit_verbatim,
                    bindings,
                ));
                continue;
            }
            AstKind::ExportFromDeclaration(d) => {
                imports.extend(declaration_readings(
                    &d.source,
                    |types| Kind::declaration(ctx, types, d.with_clause.as_deref()),
                    d.export_kind.is_type(),
                    can_emit_verbatim,
                    ctx.counted(d.specifiers.iter())
                        .map(|s| s.export_kind.is_type()),
                ));
                continue;
            }
            AstKind::ExportAllDeclaration(d) => Import::literal(
                &d.source,
                Kind::declaration(ctx, d.export_kind.is_type(), d.with_clause.as_deref()),
            ),
            AstKind::TSImportType(d) => {
                Import::literal(&d.source, type_options(ctx, d.options.as_deref()))
            }
            AstKind::TSImportEqualsDeclaration(d) => {
                let oxc::ast::ast::TSModuleReference::ExternalModuleReference(reference) =
                    &d.module_reference
                else {
                    continue;
                };
                let type_only = d.import_kind.is_type();
                imports.extend(declaration_readings(
                    &reference.expression,
                    Kind::require_declaration,
                    type_only,
                    false,
                    [Some(type_only), exported.contains(&id).then_some(true)]
                        .into_iter()
                        .flatten(),
                ));
                continue;
            }
            AstKind::ImportExpression(d) => Import::argument(&kinds, &d.source, Kind::Dynamic),
            AstKind::CallExpression(c) => match call_use(&kinds, c) {
                Some(s) => s,
                None => continue,
            },
            _ => continue,
        };
        imports.push(import);
    }
    for import in &mut imports {
        import.kind = import.kind.for_importer(in_declaration);
    }
    imports.sort_by_key(|i| i.offset);
    imports
}
