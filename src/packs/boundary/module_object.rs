use super::{BuiltinSource, values::Values};
use crate::{engine::TsCtx, packs::ts::values};
use oxc::{
    ast::{
        AstKind,
        ast::{
            CallExpression, Expression, IdentifierReference, JSXMemberExpression,
            JSXMemberExpressionObject, VariableDeclarator,
        },
    },
    span::GetSpan,
    syntax::{node::NodeId, scope::ScopeId, symbol::SymbolId},
};
use oxc_resolver::PackageType;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LoaderBase {
    Importer,
    Filename(PackageType),
    Unknown,
}

impl LoaderBase {
    pub(super) fn filename_format(self) -> Option<PackageType> {
        match self {
            Self::Filename(format) => Some(format),
            Self::Importer | Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Value {
    Loader(LoaderBase),
    Module,
    Class,
    Factory,
    Process,
    Builtin,
    Global,
    Resolve,
    OpaqueRoot,
    Data,
    Metadata,
    UnreadMember,
    Promise(BuiltinSource),
}

impl BuiltinSource {
    pub(super) fn value(self) -> Value {
        match self {
            Self::Module => Value::Class,
            Self::Process => Value::Process,
        }
    }
}
impl Value {
    pub(super) fn escapes(self) -> bool {
        matches!(
            self,
            Self::Loader(_)
                | Self::Module
                | Self::Class
                | Self::Factory
                | Self::Builtin
                | Self::Resolve
                | Self::OpaqueRoot
                | Self::UnreadMember
        )
    }

    pub(super) fn member(self, key: Option<&str>) -> Option<Self> {
        use Value::{
            Builtin, Class, Data, Factory, Global, Loader, Metadata, Module, OpaqueRoot, Process,
            Promise, Resolve, UnreadMember,
        };
        Some(match (self, key) {
            (Global | Module, Some("require")) => Loader(LoaderBase::Importer),
            (Global, Some("globalThis" | "global")) => Global,
            (Global, Some("module")) => Module,
            (Global, Some("process")) | (Process, Some("default")) => Process,
            (Process, Some("getBuiltinModule")) => Builtin,
            (Class, Some("createRequire")) => Factory,
            (Class, Some("Module" | "default")) => Class,
            (Loader(base), Some("resolve")) if base != LoaderBase::Unknown => Resolve,
            (Loader(base), Some("main")) if base != LoaderBase::Unknown => OpaqueRoot,
            (Process, Some("mainModule")) => OpaqueRoot,
            (Loader(_) | Class | Factory | Builtin, Some("name" | "length")) => Metadata,
            (Module, Some("filename" | "loaded" | "id" | "path" | "isPreloading" | "hot")) => Data,
            (Data | Metadata | UnreadMember | Promise(_), _)
            | (Global | Process, Some(_))
            | (
                Class,
                Some(
                    "builtinModules"
                    | "constants"
                    | "enableCompileCache"
                    | "findPackageJSON"
                    | "findSourceMap"
                    | "flushCompileCache"
                    | "getCompileCacheDir"
                    | "getSourceMapsSupport"
                    | "isBuiltin"
                    | "setSourceMapsSupport"
                    | "SourceMap"
                    | "stripTypeScriptTypes"
                    | "syncBuiltinESMExports",
                ),
            )
            | (Resolve, Some("paths"))
            | (Module, Some("exports")) => return None,
            _ => UnreadMember,
        })
    }
}

pub(super) struct Kinds<'ctx, 'a> {
    pub ctx: &'ctx TsCtx<'a>,
    pub values: Values<'ctx, 'a>,
    symbols: RefCell<HashMap<SymbolId, Option<Value>>>,
    nodes: RefCell<HashMap<NodeId, Option<Value>>>,
    patterns: RefCell<HashMap<NodeId, bool>>,
    function_scopes: HashSet<ScopeId>,
    runtime_globals: RefCell<HashMap<SymbolId, bool>>,
}
impl<'ctx, 'a> Kinds<'ctx, 'a> {
    pub fn new(ctx: &'ctx TsCtx<'a>) -> Self {
        let scopes = ctx.semantic.scoping();
        let mut function_scopes = HashSet::new();
        for id in ctx.counted(scopes.scope_descendants_from_root()) {
            let flags = scopes.scope_flags(id);
            if (flags.is_function() && !flags.is_arrow())
                || scopes
                    .scope_parent_id(id)
                    .is_some_and(|p| function_scopes.contains(&p))
            {
                function_scopes.insert(id);
            }
        }
        Self {
            ctx,
            values: Values::new(ctx),
            function_scopes,
            patterns: RefCell::default(),
            runtime_globals: RefCell::default(),
            nodes: RefCell::default(),
            symbols: RefCell::default(),
        }
    }
    pub fn kind_of(&self, expr: &'a Expression<'a>) -> Option<Value> {
        let nodes = self.ctx.semantic.nodes();
        let mut id = expr.node_id();
        while let Some(inner) = super::bases::wrapped_node(nodes.kind(id)) {
            self.ctx.record_work(1);
            id = inner;
        }
        self.node(id)
    }
    pub fn node(&self, id: NodeId) -> Option<Value> {
        self.ctx.record_work(1);
        let kind = self.ctx.semantic.nodes().kind(id);
        if !matches!(
            kind,
            AstKind::IdentifierReference(_)
                | AstKind::StaticMemberExpression(_)
                | AstKind::ComputedMemberExpression(_)
                | AstKind::CallExpression(_)
                | AstKind::ImportExpression(_)
                | AstKind::AwaitExpression(_)
        ) {
            return None;
        }
        if let Some(&v) = self.nodes.borrow().get(&id) {
            return v;
        }
        self.nodes.borrow_mut().insert(id, None);
        let v = self.classify(kind);
        self.nodes.borrow_mut().insert(id, v);
        v
    }
    fn classify(&self, node: AstKind<'a>) -> Option<Value> {
        match node {
            AstKind::IdentifierReference(i) => self.identifier(i),
            AstKind::StaticMemberExpression(m) => self
                .kind_of(&m.object)?
                .member(Some(m.property.name.as_str())),
            AstKind::ComputedMemberExpression(m) => {
                let object = self.kind_of(&m.object)?;
                object.member(self.values.literal(&m.expression))
            }
            AstKind::ImportExpression(e) => {
                BuiltinSource::source(self.values.literal(&e.source)).map(Value::Promise)
            }
            AstKind::AwaitExpression(e) => match self.kind_of(&e.argument)? {
                Value::Promise(source) => Some(source.value()),
                _ => None,
            },
            AstKind::CallExpression(c) => self.call(c),
            _ => None,
        }
    }
    fn identifier(&self, i: &'a IdentifierReference<'a>) -> Option<Value> {
        let scope = self.ctx.semantic.scoping();
        if i.reference_id.get().is_some_and(|r| {
            let flags = scope.get_reference(r).flags();
            flags.is_type_only() || flags.is_value_as_type()
        }) {
            return None;
        }
        if self.runtime_global(i) {
            return match i.name.as_str() {
                "arguments" => {
                    let id = scope.get_reference(i.reference_id.get()?).scope_id();
                    (!self.function_scopes.contains(&id)).then_some(Value::OpaqueRoot)
                }
                name if values::global_object(name) => Some(Value::Global),
                name => Value::Global.member(Some(name)),
            };
        }
        let symbol = values::symbol(self.ctx, i)?;
        if let Some(&value) = self.symbols.borrow().get(&symbol) {
            return value;
        }
        let value = self.binding_value(i);
        self.symbols.borrow_mut().insert(symbol, value);
        value
    }
    pub(super) fn runtime_global(&self, i: &IdentifierReference<'_>) -> bool {
        let Some(symbol) = values::symbol(self.ctx, i) else {
            return true;
        };
        if let Some(&global) = self.runtime_globals.borrow().get(&symbol) {
            return global;
        }
        let scopes = self.ctx.semantic.scoping();
        let wrapper = matches!(
            i.name.as_str(),
            "arguments" | "require" | "module" | "__filename" | "__dirname"
        ) && scopes.symbol_scope_id(symbol) == scopes.root_scope_id()
            && !self.ctx.semantic.source_type().is_module();
        let nodes = self.ctx.semantic.nodes();
        let global = self
            .ctx
            .counted(scopes.symbol_declarations(symbol))
            .all(|id| match nodes.kind(id) {
                AstKind::VariableDeclarator(v) => {
                    let AstKind::VariableDeclaration(d) = nodes.parent_kind(id) else {
                        return false;
                    };
                    d.declare || (wrapper && d.kind.is_var() && v.init.is_none())
                }
                AstKind::Function(f) => f.declare,
                AstKind::Class(c) => c.declare,
                AstKind::TSInterfaceDeclaration(_) | AstKind::TSTypeAliasDeclaration(_) => true,
                _ => false,
            });
        self.runtime_globals.borrow_mut().insert(symbol, global);
        global
    }

    fn readable_pattern(&self, v: &'a VariableDeclarator<'a>) -> bool {
        *self
            .patterns
            .borrow_mut()
            .entry(v.node_id.get())
            .or_insert_with(|| super::bases::binding_is_read(self, v))
    }

    pub(super) fn jsx_member(&self, mut member: &'a JSXMemberExpression<'a>) -> Option<Value> {
        let mut keys = Vec::new();
        let mut value = loop {
            self.ctx.record_work(1);
            keys.push(member.property.name.as_str());
            match &member.object {
                JSXMemberExpressionObject::IdentifierReference(i) => break self.identifier(i)?,
                JSXMemberExpressionObject::MemberExpression(m) => member = m,
                JSXMemberExpressionObject::ThisExpression(_) => return None,
            }
        };
        for key in self.ctx.counted(keys.into_iter().rev()) {
            value = value.member(Some(key))?;
            if value == Value::UnreadMember {
                break;
            }
        }
        Some(value)
    }
    fn binding_value(&self, i: &'a IdentifierReference<'a>) -> Option<Value> {
        if let Some(AstKind::TSImportEqualsDeclaration(import)) = self.values.declaration(i)
            && !import.import_kind.is_type()
            && let oxc::ast::ast::TSModuleReference::ExternalModuleReference(source) =
                &import.module_reference
            && let Some(root) = BuiltinSource::source(source.expression.value.as_str())
        {
            return Some(root.value());
        }
        if let Some(init) = self.values.initializer(i) {
            return self.kind_of(self.values.resolve(init));
        }
        if let Some((source, imported)) = values::import_of(self.ctx, i) {
            let root = BuiltinSource::source(Some(source))?.value();
            return match imported {
                values::Imported::Default | values::Imported::Namespace => Some(root),
                values::Imported::Named(name) => root
                    .member(Some(name))
                    .filter(|v| *v != Value::UnreadMember),
            };
        }
        let (obj, key) = self.values.bound_member(i)?;
        let AstKind::VariableDeclarator(v) = self.values.declaration(i)? else {
            return None;
        };
        self.readable_pattern(v)
            .then(|| self.kind_of(obj)?.member(Some(key)))?
            .filter(|v| *v != Value::UnreadMember)
    }
    fn call(&self, c: &'a CallExpression<'a>) -> Option<Value> {
        let arg = || c.arguments.first()?.as_expression();
        let name = match self.kind_of(&c.callee)? {
            Value::Factory => return Some(Value::Loader(loader_base(self, c))),
            Value::Builtin | Value::Loader(LoaderBase::Importer | LoaderBase::Filename(_)) => {
                self.values.literal(arg()?)
            }
            _ => return None,
        };
        BuiltinSource::source(name).map(BuiltinSource::value)
    }
    pub fn unread_alias(&self, node: AstKind<'a>) -> Option<oxc::span::Span> {
        use oxc::ast::ast::BindingPattern;
        let AstKind::VariableDeclarator(v) = node else {
            return None;
        };
        let BindingPattern::ObjectPattern(p) = &v.id else {
            return None;
        };
        if !self.readable_pattern(v) {
            return None;
        }
        let rhs = v.init.as_ref()?;
        let root = self.kind_of(rhs)?;
        self.ctx
            .counted(p.properties.iter())
            .filter_map(|prop| self.values.key(&prop.key))
            .any(|key| matches!(root.member(Some(key)), Some(Value::UnreadMember)))
            .then(|| rhs.span())
    }
    fn root_read(&self, value: Value, parent: AstKind<'a>, span: oxc::span::Span) -> bool {
        let nodes = self.ctx.semantic.nodes();
        match parent {
            AstKind::UnaryExpression(u) => u.operator.is_void(),
            AstKind::StaticMemberExpression(m) => m.object.span().contains_inclusive(span),
            AstKind::ComputedMemberExpression(m) => m.object.span().contains_inclusive(span),
            AstKind::VariableDeclarator(v) => {
                self.readable_pattern(v)
                    || (matches!(
                        nodes.parent_kind(v.node_id.get()),
                        AstKind::VariableDeclaration(d) if d.kind.is_const()
                    ) && super::bases::declared_here(self, v, &v.id))
            }
            AstKind::ExpressionStatement(_)
            | AstKind::ExportSpecifier(_)
            | AstKind::ExportDefaultDeclaration(_)
            | AstKind::TSExportAssignment(_)
            | AstKind::JSXMemberExpression(_) => true,
            AstKind::CallExpression(c) if value == Value::Global => {
                self.discarded_global_assign(c, span)
            }
            _ => false,
        }
    }

    fn object_call(&self, call: &CallExpression<'_>, names: &[&str]) -> bool {
        let Some(method) = values::unwrapped(self.ctx, &call.callee).as_member_expression() else {
            return false;
        };
        let Expression::Identifier(object) = values::unwrapped(self.ctx, method.object()) else {
            return false;
        };
        method
            .static_property_name()
            .and_then(oxc::str::JSStr::as_str)
            .is_some_and(|key| names.contains(&key))
            && object.name == "Object"
            && self.runtime_global(object)
    }

    fn discarded_global_assign(&self, call: &CallExpression<'_>, span: oxc::span::Span) -> bool {
        let [target, source] = call.arguments.as_slice() else {
            return false;
        };
        self.object_call(call, &["assign"])
            && target.span().contains_inclusive(span)
            && matches!(
                source
                    .as_expression()
                    .map(|expr| values::inner(self.ctx, expr)),
                Some(Expression::ObjectExpression(_))
            )
            && matches!(
                super::bases::outer(self.ctx, call.node_id.get()),
                Some(AstKind::ExpressionStatement(_))
            )
    }

    pub fn read(&self, id: NodeId, value: Value) -> bool {
        let nodes = self.ctx.semantic.nodes();
        let span = nodes.kind(id).span();
        let Some(parent) = super::bases::outer(self.ctx, id) else {
            return false;
        };
        let is_callee =
            matches!(parent,AstKind::CallExpression(c) if c.callee.span().contains_inclusive(span));
        if value == Value::UnreadMember {
            return matches!(parent, AstKind::CallExpression(call)
                if self.unread_member_call(nodes.kind(id), call));
        }
        if let AstKind::VariableDeclarator(v) = parent
            && matches!(
                nodes.parent_kind(nodes.parent_id(v.node_id.get())),
                AstKind::ExportDeclaration(_)
            )
        {
            match &v.id {
                oxc::ast::ast::BindingPattern::BindingIdentifier(_) if value.escapes() => {
                    return false;
                }
                oxc::ast::ast::BindingPattern::ObjectPattern(p)
                    if self.ctx.counted(p.properties.iter()).any(|p| {
                        value
                            .member(self.values.key(&p.key))
                            .is_some_and(Value::escapes)
                    }) =>
                {
                    return false;
                }
                _ => {}
            }
        }
        if matches!(value, Value::Global | Value::Process) {
            return self.root_read(value, parent, span);
        }
        if !super::bases::read_in_parent(Some(parent), span) {
            return false;
        }
        if value == Value::Data {
            return true;
        }
        if matches!(
            parent,
            AstKind::JSXMemberExpression(_) | AstKind::JSXClosingElement(_)
        ) {
            return true;
        }
        let object = match parent {
            AstKind::StaticMemberExpression(m) => Some(&m.object),
            AstKind::ComputedMemberExpression(m) => Some(&m.object),
            _ => None,
        };
        if let Some(object) = object {
            return if object.span().contains_inclusive(span) {
                !matches!(value, Value::Promise(_))
            } else {
                value == Value::Metadata
            };
        }
        if value == Value::Metadata {
            return !is_callee;
        }
        match parent {
            AstKind::CallExpression(_) => {
                is_callee
                    && matches!(
                        value,
                        Value::Factory | Value::Builtin | Value::Loader(_) | Value::Resolve
                    )
            }
            AstKind::VariableDeclarator(v) => {
                !matches!(value, Value::Promise(_))
                    && (self.readable_pattern(v) || super::bases::declared_here(self, v, &v.id))
            }
            AstKind::AwaitExpression(_) => matches!(value, Value::Promise(_)),
            AstKind::BinaryExpression(b) => b.operator.is_equality(),
            AstKind::UnaryExpression(u) => u.operator.is_typeof(),
            _ => false,
        }
    }
    fn unread_member_call(&self, node: AstKind<'a>, call: &CallExpression<'_>) -> bool {
        let (object, key) = match node {
            AstKind::StaticMemberExpression(m) => (&m.object, Some(m.property.name.as_str())),
            AstKind::ComputedMemberExpression(m) => (&m.object, self.values.literal(&m.expression)),
            _ => return false,
        };
        let span = node.span();
        match (self.kind_of(object), key) {
            (Some(Value::Loader(LoaderBase::Unknown)), Some("resolve")) => {
                call.callee.span().contains_inclusive(span)
            }
            (Some(Value::Loader(_)), Some("cache")) => {
                matches!(call.arguments.as_slice(), [arg]
                    if arg.as_expression().is_some_and(|arg| arg.span().contains_inclusive(span)))
                    && self.object_call(call, &["keys", "getOwnPropertyNames"])
            }
            _ => false,
        }
    }
}
fn loader_base<'a>(kinds: &Kinds<'_, 'a>, call: &'a CallExpression<'a>) -> LoaderBase {
    let [arg] = call.arguments.as_slice() else {
        return LoaderBase::Unknown;
    };
    let Some(expr) = arg.as_expression() else {
        return LoaderBase::Unknown;
    };
    let expr = values::unwrapped(kinds.ctx, expr);
    if let Expression::Identifier(id) = expr
        && id.name == "__filename"
        && kinds.runtime_global(id)
    {
        return LoaderBase::Filename(PackageType::CommonJs);
    }
    let Some(member) = expr.get_member_expr() else {
        return LoaderBase::Unknown;
    };
    if !matches!(
        values::unwrapped(kinds.ctx, member.object()),
        Expression::ImportMeta(_)
    ) {
        return LoaderBase::Unknown;
    }
    let key = match member {
        oxc::ast::ast::MemberExpression::ComputedMemberExpression(m) => {
            kinds.values.literal(&m.expression)
        }
        _ => member
            .static_property_name()
            .and_then(oxc::str::JSStr::as_str),
    };
    match key {
        Some("url") => LoaderBase::Importer,
        Some("filename") => LoaderBase::Filename(PackageType::Module),
        _ => LoaderBase::Unknown,
    }
}
