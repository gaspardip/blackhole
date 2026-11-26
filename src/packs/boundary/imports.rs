use super::module_object::{Kinds, LoaderBase, Value};
use crate::engine::TsCtx;
use crate::packs::ts::values::{inner, symbol};
use oxc::{
    ast::{
        AstKind,
        ast::{
            CallExpression, Declaration, Expression, ImportDeclarationSpecifier, ModuleExportName,
            ObjectExpression, ObjectPropertyKind, Statement, StringLiteral, WithClause,
        },
    },
    span::GetSpan,
    syntax::node::NodeId,
};
use oxc_resolver::PackageType;
use std::collections::HashMap;

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
    pub(super) fn in_type_context(self, type_only: bool) -> Self {
        match (type_only, self) {
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

pub(crate) struct Import {
    pub(super) offset: u32,
    pub(super) name: Option<String>,
    pub(super) kind: Kind,
    pub(super) filename_base: Option<PackageType>,
    pub(super) verbatim_only: bool,
    reexport: bool,
}

impl Import {
    pub(super) fn is_reexport(&self) -> bool {
        self.reexport
    }

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
            reexport: false,
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

#[derive(Default)]
struct ExportReadings {
    value: bool,
    types: bool,
}

fn exported_bindings(ctx: &TsCtx<'_>) -> HashMap<NodeId, ExportReadings> {
    let from_expression = |expr: &Expression<'_>| match inner(ctx, expr) {
        Expression::Identifier(id) => symbol(ctx, id),
        _ => None,
    };
    let scoping = ctx.semantic.scoping();
    let mut locals = HashMap::<_, ExportReadings>::new();
    let mut record = |binding, types| {
        if let Some(binding) = binding {
            let readings = locals
                .entry(scoping.symbol_declaration(binding))
                .or_default();
            if types {
                readings.types = true;
            } else {
                readings.value = true;
            }
        }
    };
    for stmt in ctx.counted(ctx.program.body.iter()) {
        match stmt {
            Statement::ExportNamedDeclaration(d) => {
                for specifier in ctx.counted(d.specifiers.iter()) {
                    if let ModuleExportName::IdentifierReference(id) = &specifier.local {
                        record(
                            symbol(ctx, id),
                            d.export_kind.is_type() || specifier.export_kind.is_type(),
                        );
                    }
                }
            }
            Statement::ExportDefaultDeclaration(d) => {
                record(
                    d.declaration.as_expression().and_then(from_expression),
                    false,
                );
            }
            Statement::ExportDeclaration(d) => {
                if let Declaration::TSImportEqualsDeclaration(import) = &d.declaration {
                    record(import.id.symbol_id.get(), false);
                }
            }
            Statement::TSExportAssignment(d) => record(from_expression(&d.expression), false),
            _ => {}
        }
    }
    locals
}

fn binding_readings(
    exported: &HashMap<NodeId, ExportReadings>,
    id: NodeId,
    type_only: bool,
) -> impl Iterator<Item = (bool, bool)> {
    let export = exported.get(&id);
    let reexport = export.is_some_and(|r| r.value || (type_only && r.types));
    let extra_type = !type_only && export.is_some_and(|r| r.types);
    [(type_only, reexport)]
        .into_iter()
        .chain(extra_type.then_some((true, true)))
}

fn declaration_readings(
    source: &StringLiteral<'_>,
    kind: impl Fn(bool) -> Kind,
    type_only: bool,
    can_emit_verbatim: bool,
    reexport: bool,
    bindings: impl Iterator<Item = (bool, bool)>,
) -> impl Iterator<Item = Import> {
    let mut present = [None, None];
    for (types, exported) in bindings {
        *present[usize::from(type_only || types)].get_or_insert(false) |= exported;
    }
    if present == [None, None] {
        present[usize::from(type_only)] = Some(reexport);
    }
    [false, true]
        .into_iter()
        .zip(present)
        .filter_map(move |(types, exported)| {
            Some(Import {
                reexport: exported?,
                ..Import::literal(source, kind(types))
            })
        })
        .chain(
            (can_emit_verbatim && !type_only && present[0].is_none()).then(|| Import {
                verbatim_only: true,
                reexport,
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

pub(super) fn collect(ctx: &TsCtx<'_>) -> Vec<Import> {
    let in_declaration = ctx.program.source_type.is_typescript_definition();
    let mut imports = Vec::new();
    let kinds = Kinds::new(ctx);
    let exported = exported_bindings(ctx);
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
                    let type_only = d.import_kind.is_type() || type_import(s);
                    binding_readings(&exported, s.node_id(), type_only)
                });
                imports.extend(declaration_readings(
                    &d.source,
                    |types| Kind::declaration(ctx, types, d.with_clause.as_deref()),
                    d.import_kind.is_type(),
                    can_emit_verbatim,
                    false,
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
                    true,
                    ctx.counted(d.specifiers.iter())
                        .map(|s| (s.export_kind.is_type(), true)),
                ));
                continue;
            }
            AstKind::ExportAllDeclaration(d) => Import {
                reexport: true,
                ..Import::literal(
                    &d.source,
                    Kind::declaration(ctx, d.export_kind.is_type(), d.with_clause.as_deref()),
                )
            },
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
                    false,
                    binding_readings(&exported, id, type_only),
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
        import.kind = import.kind.in_type_context(in_declaration);
    }
    imports.sort_by_key(|i| i.offset);
    imports
}
