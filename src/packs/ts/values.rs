#[path = "arguments.rs"]
mod arguments;
use arguments::Arguments;
#[path = "member_values.rs"]
pub(super) mod member_values;
pub(super) use member_values::const_initializer;
#[path = "strings.rs"]
mod strings;
#[path = "tracked_uses.rs"]
pub(super) mod tracked_uses;
use crate::engine::TsCtx;
use oxc::syntax::symbol::SymbolId;
use oxc::{
    ast::{
        AstKind,
        ast::{
            Argument, BindingPattern, CallExpression, Expression, IdentifierReference,
            MemberExpression, ObjectProperty, ObjectPropertyKind, PropertyKey,
        },
    },
    span::GetSpan,
};
use std::collections::HashSet;

use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    rc::Rc,
};
type AccumulatorAliases<'a> = HashMap<(SymbolId, SymbolId), &'a Expression<'a>>;

#[derive(Default)]
pub(crate) struct Resolution<'a> {
    pub(crate) work: Rc<Cell<u64>>,
    work_limit: OnceCell<Cell<Option<usize>>>,
    pub(super) runners: RefCell<HashMap<oxc::span::Span, super::test_context::Runner<'a>>>,
    array_callbacks: OnceCell<HashMap<oxc::span::Span, Vec<Rc<Invocation<'a>>>>>,
    pub(super) named_callbacks:
        OnceCell<HashMap<oxc::syntax::node::NodeId, Vec<super::test_context::CallbackSite<'a>>>>,
    unreadable: OnceCell<HashSet<SymbolId>>,
    assignments: OnceCell<HashMap<SymbolId, Vec<Write<'a>>>>,
    pub(super) conservative: bool,
    member_values: member_values::MemberValues<'a>,
    eval_mention: OnceCell<bool>,
    has_with: Rc<OnceCell<bool>>,
    with_bodies: OnceCell<Vec<bool>>,
    under_with_statement: OnceCell<Vec<bool>>,
    escapes: OnceCell<Vec<tracked_uses::Escape>>,
    written: OnceCell<HashSet<SymbolId>>,
    pub(super) named_exports: OnceCell<super::local_alias::NamedExports>,
    aliases: RefCell<HashMap<SymbolId, &'a Expression<'a>>>,
    accumulator_aliases: RefCell<AccumulatorAliases<'a>>,
    strings: strings::StringCache,
    objects: RefCell<HashMap<u32, Properties<'a>>>,
    pub(super) transport_options: RefCell<HashMap<u32, super::transports::TransportOption<'a>>>,
    calls: RefCell<HashMap<oxc::span::Span, Rc<InvocationTarget<'a>>>>,
}
impl Resolution<'_> {
    pub(crate) fn conservative(&self) -> Self {
        Self {
            work: Rc::clone(&self.work),
            conservative: true,
            ..Self::default()
        }
    }
    pub(super) fn limit_work(&self, steps: usize) {
        self.work_limit
            .set(Cell::new(Some(steps)))
            .expect("one work limit per rule");
    }

    pub(super) fn has_work(&self) -> bool {
        self.work_limit
            .get()
            .is_none_or(|work| work.get().is_some())
    }

    pub(super) fn record_work(&self, steps: usize) {
        self.work.set(self.work.get().saturating_add(steps as u64));
    }

    pub(super) fn take_work(&self, steps: usize) -> bool {
        self.record_work(steps);
        self.work_limit.get().is_none_or(|work| {
            let remaining = work.get().and_then(|left| left.checked_sub(steps));
            work.set(remaining);
            remaining.is_some()
        })
    }

    pub(super) fn take_string_work(&self, bytes: usize) -> bool {
        self.take_work(bytes.max(1))
    }

    pub(super) fn same_string_bytes(&self, left: &[u8], right: &[u8]) -> bool {
        left.len() == right.len()
            && (std::ptr::eq(left, right) || self.take_string_work(left.len()) && left == right)
    }
}

pub(super) fn same_key(ctx: &TsCtx<'_>, left: &str, right: &str) -> bool {
    ctx.resolution
        .same_string_bytes(left.as_bytes(), right.as_bytes())
}

pub(super) fn path_is(ctx: &TsCtx<'_>, path: &[&str], expected: &[&str]) -> bool {
    path.len() == expected.len()
        && path
            .iter()
            .zip(expected)
            .all(|(left, right)| same_key(ctx, left, right))
}

pub(super) fn has_with(ctx: &TsCtx<'_>) -> bool {
    *ctx.resolution.has_with.get_or_init(|| {
        let scoping = ctx.semantic.scoping();
        scoping.scope_descendants_from_root().any(|id| {
            ctx.resolution.record_work(1);
            scoping.scope_flags(id).is_with()
        })
    })
}

pub(super) fn symbol(ctx: &TsCtx<'_>, id: &IdentifierReference<'_>) -> Option<SymbolId> {
    ctx.semantic
        .scoping()
        .get_reference(id.reference_id.get()?)
        .symbol_id()
}
#[derive(PartialEq)]
pub(super) enum Imported<'a> {
    Named(&'a str),
    Default,
    Namespace,
}

pub(super) fn import_of<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<(&'a str, Imported<'a>)> {
    let decl = ctx.semantic.symbol_declaration(symbol(ctx, id)?);
    let AstKind::ImportDeclaration(import) = ctx.semantic.nodes().parent_kind(decl.id()) else {
        return None;
    };
    let name = match decl.kind() {
        AstKind::ImportSpecifier(s) => Imported::Named(s.imported.name().as_str()),
        AstKind::ImportDefaultSpecifier(_) => Imported::Default,
        AstKind::ImportNamespaceSpecifier(_) => Imported::Namespace,
        _ => return None,
    };
    Some((import.source.value.as_str()?, name))
}
pub(super) fn destructured<'a>(
    ctx: &TsCtx<'a>,
    pattern: &'a oxc::ast::ast::ObjectPattern<'a>,
    matches: impl Fn(&oxc::ast::ast::BindingIdentifier<'a>) -> bool,
) -> Option<&'a str> {
    let property = pattern
        .properties
        .iter()
        .find(|p| p.value.get_binding_identifier().is_some_and(&matches))?;
    key(ctx, &property.key)
}

fn unreadable(ctx: &TsCtx<'_>, sym: SymbolId) -> bool {
    ctx.resolution.unreadable.get_or_init(|| {
        let mut symbols=HashSet::new();
        let nodes=ctx.semantic.nodes();
        let scopes=ctx.semantic.scoping();
        for node in nodes.iter() {
            match node.kind() {
                AstKind::CallExpression(c) if c.callee.get_identifier_reference().is_some_and(|id|
                    same_key(ctx, id.name.as_str(), "eval") && ctx.semantic.is_reference_to_global_variable(id)) => {
                    for scope in scopes.scope_ancestors(node.scope_id()) {
                        if !ctx.resolution.take_work(1) { return symbols; }
                        for binding in scopes.iter_bindings_in(scope) {
                            if !ctx.resolution.take_work(1) { return symbols; }
                            if !scopes.symbol_flags(binding).is_const_variable() {
                                symbols.insert(binding);
                            }
                        }
                    }
                }
                AstKind::IdentifierReference(id) if id.reference_id.get().is_some_and(|r|scopes.get_reference(r).is_write()) => {
                    let plain=matches!(nodes.parent_kind(node.id()),AstKind::AssignmentExpression(a) if a.operator.is_assign() && a.left.span()==id.span);
                    if !plain || under_with_statement(ctx, node.id()) {
                        symbols.extend(symbol(ctx,id));
                    }
                }
                _=>{}
            }
        }
        symbols
    }).contains(&sym)
}

fn under_with_statement(ctx: &TsCtx<'_>, id: oxc::syntax::node::NodeId) -> bool {
    has_with(ctx)
        && ctx.resolution.under_with_statement.get_or_init(|| {
            let nodes = ctx.semantic.nodes();
            let mut under = Vec::with_capacity(nodes.len());
            // Oxc numbers parents before children, and the root is its own parent.
            for node in nodes.iter() {
                let parent = nodes.parent_id(node.id());
                let inherited = parent != node.id() && under[parent.index()];
                under.push(inherited || matches!(nodes.kind(parent), AstKind::WithStatement(_)));
            }
            under
        })[id.index()]
}

pub(super) fn declaration<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<AstKind<'a>> {
    let scope = ctx.semantic.scoping();
    let symbol = symbol(ctx, id)?;
    if ctx
        .resolution
        .written
        .get_or_init(|| {
            scope
                .symbol_ids()
                .filter(|&s| scope.symbol_is_mutated(s))
                .collect()
        })
        .contains(&symbol)
        || unreadable(ctx, symbol)
    {
        return None;
    }
    let redeclarations = scope.symbol_redeclarations(symbol);
    if redeclarations.is_empty() {
        return Some(ctx.semantic.symbol_declaration(symbol).kind());
    }

    if !ctx.resolution.conservative {
        return None;
    }
    let nodes = ctx.semantic.nodes();
    let mut implementation = None;
    for entry in redeclarations {
        if !ctx.resolution.take_work(1) {
            return None;
        }
        let kind = nodes.kind(entry.declaration);
        let AstKind::Function(function) = kind else {
            return None;
        };
        if function.declare {
            return None;
        }
        if function.body.is_some() && implementation.replace(kind).is_some() {
            return None;
        }
    }
    implementation
}

pub(super) fn initializer<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<&'a Expression<'a>> {
    let decl = declaration(ctx, id)?;
    match decl {
        AstKind::VariableDeclarator(_) => super::function_types::direct_binding(decl)?.initializer,
        AstKind::FormalParameter(p)
            if matches!(p.pattern, BindingPattern::BindingIdentifier(_)) =>
        {
            let init = p.initializer.as_ref()?;
            let function = enclosing_functions(ctx, p.node_id.get()).next()?;
            let AstKind::Function(f) = function.kind() else {
                return None;
            };
            let sym = f.id.as_ref()?.symbol_id.get()?;
            if !ctx.resolution.take_work(f.params.items.len()) {
                return None;
            }
            let index = parameter_index(&f.params, symbol(ctx, id)?)?;
            let mut refs = ctx
                .semantic
                .scoping()
                .get_resolved_references(sym)
                .peekable();
            refs.peek()?;
            let nodes = ctx.semantic.nodes();
            refs.all(|r| {
                if !ctx.resolution.take_work(1) {
                    return false;
                }
                let AstKind::CallExpression(call) = nodes.parent_kind(r.node_id()) else {
                    return false;
                };
                call.callee.span() == nodes.get_node(r.node_id()).span()
                    && call.arguments.len() <= index
                    && ctx.resolution.take_work(call.arguments.len())
                    && call
                        .arguments
                        .iter()
                        .all(|a| !matches!(a, Argument::SpreadElement(_)))
            })
            .then_some(init)
        }
        _ => None,
    }
}

struct Write<'a> {
    value: &'a Expression<'a>,
    owner: Option<oxc::syntax::node::NodeId>,
    overwritten: Option<(oxc::span::Span, Option<oxc::syntax::node::NodeId>)>,
}

fn boundary(ctx: &TsCtx<'_>, id: oxc::syntax::node::NodeId) -> Option<oxc::syntax::node::NodeId> {
    ctx.semantic
        .nodes()
        .ancestors(id)
        .take_while(|_| ctx.resolution.take_work(1))
        .find(|n| {
            matches!(
                n.kind(),
                AstKind::Function(_)
                    | AstKind::ArrowFunctionExpression(_)
                    | AstKind::ClassBody(_)
                    | AstKind::StaticBlock(_)
            )
        })
        .map(oxc::semantic::AstNode::id)
}

fn assignments<'a>(ctx: &TsCtx<'a>) -> HashMap<SymbolId, Vec<Write<'a>>> {
    use oxc::ast::ast::AssignmentTarget;
    let nodes = ctx.semantic.nodes();
    let mut by_symbol: HashMap<_, Vec<Write<'a>>> = HashMap::new();
    let mut previous = HashMap::new();
    for node in nodes.iter().take_while(|_| ctx.resolution.take_work(1)) {
        let written = match node.kind() {
            AstKind::VariableDeclarator(v) => {
                v.id.get_binding_identifier()
                    .and_then(|id| id.symbol_id.get())
                    .zip(v.init.as_ref())
            }
            AstKind::AssignmentExpression(a) if a.operator.is_assign() => {
                if let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                    symbol(ctx, id).map(|sym| (sym, &a.right))
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some((sym, value)) = written else {
            continue;
        };
        let writes = by_symbol.entry(sym).or_default();
        let statement = nodes.parent_node(node.id());
        let list = nodes.parent_node(statement.id());
        if matches!(
            statement.kind(),
            AstKind::ExpressionStatement(_) | AstKind::VariableDeclaration(_)
        ) && matches!(
            list.kind(),
            AstKind::Program(_)
                | AstKind::BlockStatement(_)
                | AstKind::FunctionBody(_)
                | AstKind::SwitchCase(_)
        ) && let Some(index) = previous.insert((sym, list.id()), writes.len())
        {
            let range = oxc::span::Span::new(statement.span().end, list.span().end);
            let old: &mut Write = &mut writes[index];
            old.overwritten = Some((range, boundary(ctx, list.id())));
        }
        writes.push(Write {
            value,
            owner: boundary(ctx, node.id()),
            overwritten: None,
        });
    }
    by_symbol
}

fn live_writes<'a>(ctx: &TsCtx<'a>, id: &IdentifierReference<'a>) -> Vec<&'a Expression<'a>> {
    let Some(writes) = symbol(ctx, id).and_then(|sym| {
        ctx.resolution
            .assignments
            .get_or_init(|| assignments(ctx))
            .get(&sym)
    }) else {
        return Vec::new();
    };
    let owner = OnceCell::new();
    let loops = OnceCell::new();
    writes
        .iter()
        .take_while(|_| ctx.resolution.take_work(1))
        .filter(|w| {
            let owner = || *owner.get_or_init(|| boundary(ctx, id.node_id.get()));
            (w.value.span().end <= id.span.start
                || w.owner != owner()
                || loops
                    .get_or_init(|| {
                        ctx.semantic
                            .nodes()
                            .ancestors(id.node_id.get())
                            .take_while(|_| ctx.resolution.take_work(1))
                            .take_while(|node| Some(node.id()) != owner())
                            .filter(|node| node.kind().is_iteration_statement())
                            .map(oxc::semantic::AstNode::span)
                            .collect::<Vec<_>>()
                    })
                    .iter()
                    .take_while(|_| ctx.resolution.take_work(1))
                    .any(|span| span.contains_inclusive(w.value.span())))
                && !w
                    .overwritten
                    .is_some_and(|(range, b)| range.contains_inclusive(id.span) && owner() == b)
        })
        .map(|w| w.value)
        .collect()
}

pub(super) fn any_value<'a>(
    ctx: &TsCtx<'a>,
    initial: &'a Expression<'a>,
    mut matches: impl FnMut(&'a Expression<'a>) -> bool,
) -> bool {
    let mut pending = vec![initial];
    let mut seen = HashSet::new();
    while let Some(value) = pending.pop() {
        let value = resolve(ctx, value);
        if !seen.insert(value.span()) {
            continue;
        }
        if matches(value) {
            return true;
        }
        match value {
            Expression::Identifier(id) => pending.extend(live_writes(ctx, id)),
            Expression::ConditionalExpression(c) => pending.extend([&c.consequent, &c.alternate]),
            _ => (),
        }
    }
    false
}

pub(super) fn any_assigned_path<'a>(
    ctx: &TsCtx<'a>,
    initial: &'a Expression<'a>,
    matches: impl Fn(Origin, &[&str]) -> bool,
) -> bool {
    any_value(ctx, initial, |e| {
        origin_path(ctx, e).is_some_and(|(origin, path)| matches(origin, &path))
    })
}

pub(super) fn unwrapped<'a>(ctx: &TsCtx<'_>, mut expr: &'a Expression<'a>) -> &'a Expression<'a> {
    loop {
        if !ctx.resolution.take_work(1) {
            return expr;
        }
        expr = inner(ctx, expr);
        if let Expression::ChainExpression(chain) = expr
            && let oxc::ast::ast::ChainElement::TSNonNullExpression(n) = &chain.expression
        {
            expr = &n.expression;
            continue;
        }
        if let Expression::SequenceExpression(seq) = expr
            && let Some(last) = seq.expressions.last()
        {
            expr = last;
            continue;
        }
        return expr;
    }
}

pub(super) fn inner<'a>(ctx: &TsCtx<'_>, mut expr: &'a Expression<'a>) -> &'a Expression<'a> {
    while ctx.resolution.take_work(1) {
        expr = match expr {
            Expression::ParenthesizedExpression(n) => &n.expression,
            Expression::TSAsExpression(n) => &n.expression,
            Expression::TSSatisfiesExpression(n) => &n.expression,
            Expression::TSInstantiationExpression(n) => &n.expression,
            Expression::TSNonNullExpression(n) => &n.expression,
            Expression::TSTypeAssertion(n) => &n.expression,
            _ => break,
        };
    }
    expr
}

pub(super) fn binding_value<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> &'a Expression<'a> {
    binding_value_until(ctx, expr, None)
}

pub(super) fn binding_value_until<'a>(
    ctx: &TsCtx<'a>,
    mut expr: &'a Expression<'a>,
    stop: Option<SymbolId>,
) -> &'a Expression<'a> {
    let mut seen = HashSet::new();
    let mut aliases = Vec::new();
    loop {
        expr = unwrapped(ctx, expr);
        if !ctx.resolution.take_work(1) {
            return expr;
        }
        let Expression::Identifier(id) = expr else {
            break;
        };
        let sym = symbol(ctx, id);
        if stop.is_some() && sym == stop {
            break;
        }
        if let Some(sym) = sym {
            let cached = match stop {
                Some(stop) => ctx
                    .resolution
                    .accumulator_aliases
                    .borrow()
                    .get(&(stop, sym))
                    .copied(),
                None => ctx.resolution.aliases.borrow().get(&sym).copied(),
            };
            if let Some(cached) = cached {
                expr = cached;
                break;
            }
        }
        if !seen.insert(id.span.start) {
            return expr;
        }
        if let Some(next) = initializer(ctx, id) {
            aliases.extend(sym);
            expr = next;
        } else if ctx.resolution.has_work()
            && sym.is_some_and(|s| !unreadable(ctx, s))
            && declaration(ctx, id).is_none()
            && let [next] = live_writes(ctx, id).as_slice()
        {
            // A bounded lookup should not cache aliases across a time-dependent write.
            if stop.is_some() {
                aliases.clear();
            }
            expr = next;
        } else {
            break;
        }
    }
    if let Some(stop) = stop {
        if matches!(expr, Expression::Identifier(id) if symbol(ctx, id) == Some(stop)) {
            ctx.resolution
                .accumulator_aliases
                .borrow_mut()
                .extend(aliases.into_iter().map(|s| ((stop, s), expr)));
        }
    } else {
        ctx.resolution
            .aliases
            .borrow_mut()
            .extend(aliases.into_iter().map(|s| (s, expr)));
    }
    expr
}

fn projection<'a>(
    ctx: &TsCtx<'a>,
    object: &'a Expression<'a>,
    name: &str,
) -> Option<&'a oxc::ast::ast::ObjectProperty<'a>> {
    let Expression::ObjectExpression(obj) = binding_value(ctx, object) else {
        return None;
    };
    for field in obj.properties.iter().rev() {
        let ObjectPropertyKind::ObjectProperty(p) = field else {
            return None;
        };
        if p.key.static_name()? == name {
            return (p.kind == oxc::ast::ast::PropertyKind::Init).then_some(p);
        }
    }
    None
}

fn instance_field<'a>(
    ctx: &TsCtx<'a>,
    object: &'a Expression<'a>,
    name: &str,
) -> Option<&'a Expression<'a>> {
    let Expression::NewExpression(new) = binding_value(ctx, object) else {
        return None;
    };
    let id = binding_value(ctx, &new.callee).get_identifier_reference()?;
    let AstKind::Class(class) = declaration(ctx, id)? else {
        return None;
    };
    if class.heritage.is_some() {
        return None;
    }
    let mut value = None;
    for element in &class.body.body {
        let oxc::ast::ast::ClassElement::PropertyDefinition(field) = element else {
            return None;
        };
        if !field.r#static && field.key.static_name()? == name {
            value = field.value.as_ref();
        }
    }
    value
}

pub(super) fn resolve<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> &'a Expression<'a> {
    resolve_with(ctx, expr, binding_value)
}

pub(super) fn resolve_with<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
    binding: impl Fn(&TsCtx<'a>, &'a Expression<'a>) -> &'a Expression<'a>,
) -> &'a Expression<'a> {
    let mut value = binding(ctx, expr);
    let mut seen = HashSet::new();
    while ctx.resolution.take_work(1)
        && let Some(member) = value.get_member_expr()
    {
        let next = if ctx.resolution.conservative {
            member_values::value(ctx, member)
        } else {
            member.static_property_name().and_then(|key| {
                let key = key.as_str()?;
                projection(ctx, member.object(), key)
                    .map(|p| &p.value)
                    .or_else(|| instance_field(ctx, member.object(), key))
            })
        };
        let Some(next) = next else { break };
        if !seen.insert(next.span()) {
            break;
        }
        value = binding(ctx, next);
    }
    value
}

pub(super) fn alternatives<'a>(
    expr: &'a Expression<'a>,
) -> impl Iterator<Item = &'a Expression<'a>> {
    let branches = match expr.get_inner_expression() {
        Expression::AwaitExpression(a) => [Some(&a.argument), None],
        Expression::ConditionalExpression(c) => [Some(&c.consequent), Some(&c.alternate)],
        Expression::LogicalExpression(l) => [Some(&l.left), Some(&l.right)],
        Expression::SequenceExpression(s) => [s.expressions.last(), None],
        _ => [None, None],
    };
    branches.into_iter().flatten()
}

pub(super) fn resolves_to_global<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
    name: &str,
) -> bool {
    let Expression::Identifier(id) = resolve(ctx, expr) else {
        return false;
    };
    same_key(ctx, id.name.as_str(), name) && ctx.semantic.is_reference_to_global_variable(id)
}

pub(super) use strings::{contains_shape, static_string_contains_shape};

pub(super) fn literal<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<&'a str> {
    match resolve(ctx, expr) {
        Expression::StringLiteral(s) => s.value.as_str(),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
            t.quasis.first()?.value.cooked?.as_str()
        }
        _ => None,
    }
}

pub(super) fn key<'a>(ctx: &TsCtx<'a>, key: &'a PropertyKey<'a>) -> Option<&'a str> {
    if let PropertyKey::StaticIdentifier(id) = key {
        return Some(id.name.as_str());
    }
    let expr = key.as_expression()?;
    if let Expression::NumericLiteral(n) = expr {
        return n.raw.as_ref().map(oxc::str::Str::as_str);
    }
    literal(ctx, expr)
}

pub(super) struct MemberKeys<'ctx, 'a>(pub(super) &'ctx TsCtx<'a>);
impl<'a> MemberKeys<'_, 'a> {
    pub(super) fn read(&self, member: &'a MemberExpression<'a>) -> Option<&'a str> {
        use oxc::ast::MemberExpressionKind as Kind;
        self.read_kind(match member {
            MemberExpression::StaticMemberExpression(m) => Kind::Static(m),
            MemberExpression::ComputedMemberExpression(m) => Kind::Computed(m),
            MemberExpression::PrivateFieldExpression(m) => Kind::PrivateField(m),
        })
    }
    pub(super) fn read_kind(&self, member: oxc::ast::MemberExpressionKind<'a>) -> Option<&'a str> {
        match (self, member) {
            (Self(ctx), oxc::ast::MemberExpressionKind::Computed(m)) => {
                if ctx.resolution.conservative {
                    member_values::selector_key(ctx, &m.expression)?.as_str()
                } else {
                    literal(ctx, &m.expression)
                }
            }
            (_, m) => m.static_property_name()?.as_str(),
        }
    }
}

pub(super) fn path<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<Vec<&'a str>> {
    Some(origin_path(ctx, expr)?.1)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Origin {
    Global,
    Module,
    Required,
}

pub(super) fn required_module<'a>(
    ctx: &TsCtx<'a>,
    call: &'a CallExpression<'a>,
) -> Option<&'a str> {
    if !resolves_to_global(ctx, &call.callee, "require") {
        return None;
    }
    let [arg] = call.arguments.as_slice() else {
        return None;
    };
    literal(ctx, arg.as_expression()?)
}

pub(super) fn origin_path<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
) -> Option<(Origin, Vec<&'a str>)> {
    let mut origin = Origin::Global;
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    let mut expr = expr;
    loop {
        if !ctx.resolution.take_work(1) {
            return None;
        }
        expr = resolve(ctx, expr);
        if !ctx.resolution.has_work() || !seen.insert(expr.span()) {
            return None;
        }
        if let Some((object, key)) = receiver(ctx, expr) {
            parts.push(key);
            expr = object;
            continue;
        }
        if let Some(call) = call_expr(expr) {
            parts.push(required_module(ctx, call)?);
            origin = Origin::Required;
            break;
        }
        if let Expression::AwaitExpression(wait) = expr
            && let Expression::ImportExpression(import) = resolve(ctx, &wait.argument)
        {
            parts.push(literal(ctx, &import.source)?);
            origin = Origin::Module;
            break;
        }
        if matches!(expr, Expression::ImportMeta(_)) {
            parts.push("import.meta");
            break;
        }
        let Expression::Identifier(id) = expr else {
            return None;
        };
        if ctx.semantic.is_reference_to_global_variable(id) {
            let name = id.name.as_str();
            if !(name.len() <= "globalThis".len()
                && ctx.resolution.take_string_work(name.len())
                && global_object(name))
            {
                parts.push(name);
            }
            break;
        }
        let decl = ctx.semantic.symbol_declaration(symbol(ctx, id)?);
        if let AstKind::ImportDeclaration(import) = ctx.semantic.nodes().parent_kind(decl.id())
            && (import.import_kind.is_type()
                || matches!(decl.kind(),AstKind::ImportSpecifier(spec) if spec.import_kind.is_type()))
        {
            return None;
        }
        origin = Origin::Module;
        let (source, imported) = import_of(ctx, id)?;
        match imported {
            Imported::Named(name) => parts.push(name),
            Imported::Default => parts.push("default"),
            Imported::Namespace => (),
        }
        parts.push(source);
        break;
    }
    parts.reverse();
    if matches!(
        parts.as_slice(),
        ["node:timers" | "timers", "setTimeout" | "setInterval"]
    ) {
        parts.remove(0);
    }
    Some((origin, parts))
}

pub(super) fn enclosing_functions<'ctx, 'a>(
    ctx: &'ctx TsCtx<'a>,
    id: oxc::syntax::node::NodeId,
) -> impl Iterator<Item = &'ctx oxc::semantic::AstNode<'a>> {
    ctx.semantic
        .nodes()
        .ancestors(id)
        .take_while(|_| ctx.resolution.take_work(1))
        .filter(|node| {
            matches!(
                node.kind(),
                AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
            )
        })
}

pub(super) fn parameter_index(
    params: &oxc::ast::ast::FormalParameters<'_>,
    symbol: SymbolId,
) -> Option<usize> {
    params.items.iter().position(|p| {
        p.pattern
            .get_binding_identifier()
            .is_some_and(|id| id.symbol_id.get() == Some(symbol))
    })
}

pub(super) fn callback_call<'a>(
    ctx: &TsCtx<'a>,
    symbol: SymbolId,
    first_parameters: usize,
) -> Option<Rc<Invocation<'a>>> {
    let function = enclosing_functions(ctx, ctx.semantic.symbol_declaration(symbol).id()).next()?;
    let params = match function.kind() {
        AstKind::Function(f) => &f.params,
        AstKind::ArrowFunctionExpression(f) => &f.params,
        _ => return None,
    };
    if parameter_index(params, symbol)? >= first_parameters {
        return None;
    }
    let call = enclosing_call(ctx, function.id())?;
    call.arg(1)
        .is_some_and(|e| resolve(ctx, e).span() == function.span())
        .then_some(call)
}

pub(super) fn enclosing_call<'a>(
    ctx: &TsCtx<'a>,
    id: oxc::syntax::node::NodeId,
) -> Option<Rc<Invocation<'a>>> {
    match outer(ctx, id)? {
        kind @ (AstKind::CallExpression(_)
        | AstKind::NewExpression(_)
        | AstKind::TaggedTemplateExpression(_)) => site(ctx, kind).map(|s| s.call),
        AstKind::ArrayExpression(_) => {
            let callbacks = ctx.resolution.array_callbacks.get_or_init(|| {
                let mut callbacks = HashMap::<_, Vec<_>>::new();
                for node in ctx.semantic.nodes().iter() {
                    let Some(site) = site(ctx, node.kind()) else {
                        continue;
                    };
                    for arg in site.call.args.iter().flatten() {
                        let value = unwrapped(ctx, arg);
                        if value.is_function() {
                            callbacks
                                .entry(value.span())
                                .or_default()
                                .push(Rc::clone(&site.call));
                        }
                    }
                }
                callbacks
            });
            let calls = callbacks.get(&ctx.semantic.nodes().get_node(id).span())?;
            calls
                .iter()
                .find(|c| super::test_context::runner(ctx, c).test_or_suite())
                .or_else(|| calls.first())
                .cloned()
        }
        _ => None,
    }
}

pub(super) fn outer<'a>(ctx: &TsCtx<'a>, id: oxc::syntax::node::NodeId) -> Option<AstKind<'a>> {
    ctx.semantic
        .nodes()
        .ancestors(id)
        .take_while(|_| ctx.resolution.take_work(1))
        .map(oxc::semantic::AstNode::kind)
        .find(|kind| !transparent(kind))
}

/// Wrappers that leave the wrapped expression's value and target unchanged.
fn transparent(kind: &AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::ParenthesizedExpression(_)
            | AstKind::ChainExpression(_)
            | AstKind::TSAsExpression(_)
            | AstKind::TSSatisfiesExpression(_)
            | AstKind::TSInstantiationExpression(_)
            | AstKind::TSNonNullExpression(_)
            | AstKind::TSTypeAssertion(_)
    )
}

pub(super) fn global_object(name: &str) -> bool {
    matches!(name, "window" | "self" | "global" | "globalThis")
}

pub(super) fn any_property<'a>(
    ctx: &TsCtx<'a>,
    initial: &'a Expression<'a>,
    pred: impl Fn(&'a oxc::ast::ast::ObjectProperty<'a>) -> bool,
) -> bool {
    let mut pending = vec![initial];
    let mut seen = HashSet::new();
    while let Some(expr) = pending.pop() {
        let expr = resolve(ctx, expr);
        if !seen.insert(expr.span()) {
            continue;
        }
        match expr {
            Expression::ObjectExpression(obj) => {
                for p in &obj.properties {
                    match p {
                        ObjectPropertyKind::SpreadProperty(s) => pending.push(&s.argument),
                        ObjectPropertyKind::ObjectProperty(p) if pred(p) => {
                            return true;
                        }
                        ObjectPropertyKind::ObjectProperty(_) => (),
                    }
                }
            }
            _ => pending.extend(alternatives(expr)),
        }
    }
    false
}

pub(super) enum ThisArg<'a> {
    Member,
    Explicit(Option<&'a Expression<'a>>),
}

pub(super) struct InvocationTarget<'a> {
    pub(super) this_arg: ThisArg<'a>,
    origin: Option<Origin>,
    pub(super) callee: &'a Expression<'a>,
    path: Option<Vec<&'a str>>,
    pub(super) function_apply: bool,
    pending: Cell<(Arguments<'a>, Vec<oxc::span::Span>)>,
    expanded: OnceCell<Rc<Invocation<'a>>>,
}

impl<'a> InvocationTarget<'a> {
    pub(super) fn global_path(&self) -> Option<&[&'a str]> {
        self.path
            .as_deref()
            .filter(|_| self.origin == Some(Origin::Global))
    }

    pub(super) fn expand(&self, ctx: &TsCtx<'a>, template_arg: bool) -> Rc<Invocation<'a>> {
        Rc::clone(self.expanded.get_or_init(|| {
            let (pending, intrinsic_targets) = self.pending.take();
            let args = pending.finish(ctx);
            Rc::new(Invocation {
                callee: self.callee,
                path: self.path.clone(),
                unreadable_args: args.unreadable,
                template_arg,
                intrinsic_targets,
                args: args.values,
                positional: args.positional,
            })
        }))
    }
}

pub(super) fn invocation_target<'a>(
    ctx: &TsCtx<'a>,
    call: &'a CallExpression<'a>,
) -> Option<Rc<InvocationTarget<'a>>> {
    cached_target(ctx, call.span, &call.callee, call.arguments.len(), || {
        Arguments::new(&call.arguments)
    })
}

pub(super) struct Invocation<'a> {
    pub(super) callee: &'a Expression<'a>,
    pub(super) path: Option<Vec<&'a str>>,

    pub(super) unreadable_args: Option<oxc::span::Span>,
    pub(super) template_arg: bool,
    pub(super) intrinsic_targets: Vec<oxc::span::Span>,
    pub(super) args: Vec<Option<&'a Expression<'a>>>,
    positional: usize,
}
impl<'a> Invocation<'a> {
    pub(super) fn positional(&self) -> &[Option<&'a Expression<'a>>] {
        &self.args[..self.positional]
    }

    pub(super) fn arg(&self, i: usize) -> Option<&'a Expression<'a>> {
        self.args.get(i).copied().flatten()
    }
}
pub(super) fn call_position<'a>(ctx: &TsCtx<'a>, call: &'a CallExpression<'a>) -> usize {
    if origin_path(ctx, &call.callee)
        .is_some_and(|(origin, path)| origin == Origin::Global && path == ["Reflect", "apply"])
        && let Some(target) = call
            .arguments
            .first()
            .and_then(Argument::as_expression)
            .map(|expr| unwrapped(ctx, expr))
    {
        return target
            .get_member_expr()
            .map_or_else(|| target.span(), |member| member_span(ctx, member))
            .start as usize;
    }
    call.span.start as usize
}
pub(super) fn invoked<'a>(ctx: &TsCtx<'a>, call: &'a CallExpression<'a>) -> Rc<Invocation<'a>> {
    invocation_target(ctx, call)
        .expect("unlimited resolution")
        .expand(ctx, false)
}

pub(super) struct Site<'a> {
    pub(super) span: oxc::span::Span,
    pub(super) callee: &'a Expression<'a>,
    pub(super) call: Rc<Invocation<'a>>,
}

pub(super) fn site<'a>(ctx: &TsCtx<'a>, node: AstKind<'a>) -> Option<Site<'a>> {
    let (callee, target) = site_target(ctx, node)?;
    let call = target.expand(ctx, matches!(node, AstKind::TaggedTemplateExpression(_)));
    Some(Site {
        span: node.span(),
        callee,
        call,
    })
}

pub(super) fn site_target<'a>(
    ctx: &TsCtx<'a>,
    node: AstKind<'a>,
) -> Option<(&'a Expression<'a>, Rc<InvocationTarget<'a>>)> {
    Some(match node {
        AstKind::CallExpression(c) => (&c.callee, invocation_target(ctx, c)?),
        AstKind::NewExpression(n) => (
            &n.callee,
            cached_target(ctx, n.span, &n.callee, n.arguments.len(), || {
                Arguments::new(&n.arguments)
            })?,
        ),
        AstKind::TaggedTemplateExpression(t) => (
            &t.tag,
            cached_target(ctx, t.span, &t.tag, t.quasi.expressions.len(), || {
                Arguments::tagged(&t.quasi.expressions)
            })?,
        ),
        _ => return None,
    })
}

fn cached_target<'a>(
    ctx: &TsCtx<'a>,
    span: oxc::span::Span,
    target: &'a Expression<'a>,
    arg_count: usize,
    args: impl FnOnce() -> Arguments<'a>,
) -> Option<Rc<InvocationTarget<'a>>> {
    let calls = &ctx.resolution.calls;
    if let Some(hit) = calls.borrow().get(&span) {
        return Some(Rc::clone(hit));
    }
    if !ctx.resolution.take_work(arg_count) {
        return None;
    }
    let result = Rc::new(resolve_invocation(ctx, target, args())?);
    calls.borrow_mut().insert(span, Rc::clone(&result));
    Some(result)
}

fn resolve_invocation<'a>(
    ctx: &TsCtx<'a>,
    target: &'a Expression<'a>,
    mut args: Arguments<'a>,
) -> Option<InvocationTarget<'a>> {
    let mut callee = resolve(ctx, target);
    let mut intrinsic_targets = Vec::new();
    let mut seen = HashSet::new();
    let mut this_arg = ThisArg::Member;
    let mut function_apply = false;
    let (origin, path) = loop {
        if !ctx.resolution.take_work(1) {
            return None;
        }
        let (origin, parts) = origin_path(ctx, callee).unzip();
        if !ctx.resolution.has_work() {
            return None;
        }

        if !seen.insert(callee.span()) {
            break (origin, parts);
        }
        if origin == Some(Origin::Global)
            && parts
                .as_deref()
                .is_some_and(|path| path_is(ctx, path, &["Function", "prototype", "call", "call"]))
            && let Some(target) = args.next_expression(ctx)
        {
            intrinsic_targets.push(target.span());
            callee = resolve(ctx, target);
            this_arg = ThisArg::Explicit(args.next_expression(ctx));
            continue;
        }
        if let Some(c) = call_expr(callee)
            && let written = resolve(ctx, &c.callee)
            && ctx.resolution.has_work()
            && let Some(m) = written.get_member_expr()
            && MemberKeys(ctx)
                .read(m)
                .is_some_and(|key| same_key(ctx, key, "bind"))
        {
            if !ctx.resolution.take_work(c.arguments.len()) {
                return None;
            }
            args.prepend(&c.arguments);
            let bound_this = args.next_expression(ctx);
            if origin_path(ctx, m.object()).is_some_and(|(origin, path)| {
                origin == Origin::Global && path_is(ctx, &path, &["Function", "prototype", "call"])
            }) && let Some(target) = bound_this
            {
                intrinsic_targets.push(target.span());
                callee = resolve(ctx, target);
                this_arg = ThisArg::Explicit(args.next_expression(ctx));
            } else {
                this_arg = ThisArg::Explicit(bound_this);
                callee = resolve(ctx, m.object());
            }
            continue;
        }
        if origin == Some(Origin::Global)
            && parts
                .as_deref()
                .is_some_and(|path| path_is(ctx, path, &["Reflect", "apply"]))
            && let Some(target) = args.next_expression(ctx)
        {
            this_arg = ThisArg::Explicit(args.next_expression(ctx));
            args = Arguments::array(args.next_expression(ctx));
            intrinsic_targets.push(target.span());
            callee = resolve(ctx, target);
            continue;
        }
        if !ctx.resolution.has_work() {
            return None;
        }
        let Some(m) = callee.get_member_expr() else {
            break (origin, parts);
        };
        match MemberKeys(ctx).read(m) {
            Some(key) if same_key(ctx, key, "call") => {
                this_arg = ThisArg::Explicit(args.next_expression(ctx));
            }
            Some(key) if same_key(ctx, key, "apply") => {
                function_apply = true;
                this_arg = ThisArg::Explicit(args.next_expression(ctx));
                args = Arguments::array(args.next_expression(ctx));
            }
            _ => break (origin, parts),
        }
        callee = resolve(ctx, m.object());
    };
    if !ctx.resolution.has_work() {
        return None;
    }
    Some(InvocationTarget {
        this_arg,
        origin,
        callee,
        path,
        function_apply,
        pending: Cell::new((args, intrinsic_targets)),
        expanded: OnceCell::new(),
    })
}

pub(super) fn call_expr<'a>(e: &'a Expression<'a>) -> Option<&'a CallExpression<'a>> {
    match e.get_inner_expression() {
        Expression::CallExpression(c) => Some(c),
        Expression::ChainExpression(c) => match &c.expression {
            oxc::ast::ast::ChainElement::CallExpression(c) => Some(c),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone, Default)]
pub(super) struct Properties<'a> {
    pub(super) named: std::collections::BTreeMap<&'a str, &'a ObjectProperty<'a>>,
    pub(super) unknown: bool,
}
pub(super) trait ObjectSummary<'a>: Clone + Default {
    fn field(&mut self, ctx: &TsCtx<'a>, property: &'a ObjectProperty<'a>);
    fn unreadable(&mut self);
    fn overlay(&mut self, other: &Self);
}
impl<'a> ObjectSummary<'a> for Properties<'a> {
    fn field(&mut self, ctx: &TsCtx<'a>, property: &'a ObjectProperty<'a>) {
        if let Some(name) = key(ctx, &property.key) {
            self.named.insert(name, property);
        } else {
            self.unreadable();
        }
    }
    fn unreadable(&mut self) {
        self.unknown = true;
        self.named.clear();
    }
    fn overlay(&mut self, other: &Self) {
        if other.unknown {
            self.unreadable();
        }
        self.named.extend(
            other
                .named
                .iter()
                .map(|(&name, &property)| (name, property)),
        );
    }
}
pub(super) fn properties<'a>(
    ctx: &TsCtx<'a>,
    initial: &'a Expression<'a>,
) -> Option<Properties<'a>> {
    object_summary(ctx, initial, &ctx.resolution.objects)
}
pub(super) fn object_summary<'a, S: ObjectSummary<'a>>(
    ctx: &TsCtx<'a>,
    initial: &'a Expression<'a>,
    cache: &RefCell<HashMap<u32, S>>,
) -> Option<S> {
    use oxc::ast::ast::ObjectExpression;
    struct Frame<'a, S> {
        object: &'a ObjectExpression<'a>,
        index: usize,
        summary: S,
        cyclic: bool,
    }
    let Expression::ObjectExpression(root) = resolve(ctx, initial) else {
        return None;
    };
    let mut cache = cache.borrow_mut();
    if let Some(hit) = cache.get(&root.span.start) {
        return Some(hit.clone());
    }
    let mut active = HashSet::from([root.span.start]);
    let mut stack = vec![Frame {
        object: root,
        index: 0,
        summary: S::default(),
        cyclic: false,
    }];
    loop {
        let frame = stack.last_mut()?;
        let Some(property) = frame.object.properties.get(frame.index) else {
            let finished = stack.pop()?;
            let start = finished.object.span.start;
            active.remove(&start);
            if !finished.cyclic {
                cache.insert(start, finished.summary.clone());
            }
            let Some(parent) = stack.last_mut() else {
                return Some(finished.summary);
            };
            parent.cyclic |= finished.cyclic;
            parent.summary.overlay(&finished.summary);
            continue;
        };
        frame.index += 1;
        match property {
            ObjectPropertyKind::ObjectProperty(p) => frame.summary.field(ctx, p),
            ObjectPropertyKind::SpreadProperty(s) => {
                let Expression::ObjectExpression(child) = resolve(ctx, &s.argument) else {
                    frame.summary.unreadable();
                    continue;
                };
                if let Some(hit) = cache.get(&child.span.start) {
                    frame.summary.overlay(hit);
                } else if active.insert(child.span.start) {
                    stack.push(Frame {
                        object: child,
                        index: 0,
                        summary: S::default(),
                        cyclic: false,
                    });
                } else {
                    frame.cyclic = true;
                    frame.summary.unreadable();
                }
            }
        }
    }
}

pub(super) fn receiver<'a>(
    ctx: &TsCtx<'a>,
    callee: &'a Expression<'a>,
) -> Option<(&'a Expression<'a>, &'a str)> {
    let callee = inner(ctx, callee);
    if !ctx.resolution.take_work(1) {
        return None;
    }
    if let Some(m) = callee.get_member_expr() {
        return Some((m.object(), MemberKeys(ctx).read(m)?));
    }
    let id = callee.get_identifier_reference()?;
    let AstKind::VariableDeclarator(v) = declaration(ctx, id)? else {
        return None;
    };
    let BindingPattern::ObjectPattern(pattern) = &v.id else {
        return None;
    };
    if !ctx.resolution.take_work(pattern.properties.len()) {
        return None;
    }
    let symbol = symbol(ctx, id)?;
    Some((
        v.init.as_ref()?,
        destructured(ctx, pattern, |b| b.symbol_id.get() == Some(symbol))?,
    ))
}

pub(super) fn member_span<'a>(
    ctx: &TsCtx<'a>,
    member: &'a MemberExpression<'a>,
) -> oxc::span::Span {
    match member {
        MemberExpression::StaticMemberExpression(m) => m.property.span,
        MemberExpression::ComputedMemberExpression(m) => resolve(ctx, &m.expression).span(),
        MemberExpression::PrivateFieldExpression(m) => m.field.span,
    }
}

pub(super) fn member_object(member: oxc::ast::MemberExpressionKind<'_>) -> &Expression<'_> {
    use oxc::ast::MemberExpressionKind as Kind;
    match member {
        Kind::Computed(m) => &m.object,
        Kind::Static(m) => &m.object,
        Kind::PrivateField(m) => &m.object,
    }
}
