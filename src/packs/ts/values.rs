use crate::engine::TsCtx;
use oxc::syntax::symbol::SymbolId;
use oxc::{
    ast::{
        AstKind,
        ast::{
            Argument, BindingPattern, CallExpression, Expression, IdentifierReference,
            MemberExpression, ObjectPropertyKind, PropertyKey,
        },
    },
    span::GetSpan,
};
use std::collections::HashSet;

use std::{
    cell::{OnceCell, RefCell},
    collections::HashMap,
    rc::Rc,
};
#[derive(Default)]
pub(crate) struct Resolution<'a> {
    pub(super) runners: RefCell<HashMap<oxc::span::Span, super::test_context::Runner<'a>>>,
    written: OnceCell<HashSet<SymbolId>>,
    aliases: RefCell<HashMap<SymbolId, &'a Expression<'a>>>,
    calls: RefCell<HashMap<oxc::span::Span, Rc<Invocation<'a>>>>,
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

pub(super) fn declaration<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<AstKind<'a>> {
    let scope = ctx.semantic.scoping();
    let symbol = symbol(ctx, id)?;
    (!ctx
        .resolution
        .written
        .get_or_init(|| {
            scope
                .symbol_ids()
                .filter(|&s| scope.symbol_is_mutated(s))
                .collect()
        })
        .contains(&symbol)
        && scope.symbol_redeclarations(symbol).is_empty())
    .then(|| ctx.semantic.symbol_declaration(symbol).kind())
}

pub(super) fn initializer<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<&'a Expression<'a>> {
    match declaration(ctx, id)? {
        AstKind::VariableDeclarator(v) if matches!(v.id, BindingPattern::BindingIdentifier(_)) => {
            v.init.as_ref()
        }
        _ => None,
    }
}

pub(super) fn unwrapped<'a>(mut expr: &'a Expression<'a>) -> &'a Expression<'a> {
    loop {
        expr = expr.get_inner_expression();
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

pub(super) fn resolve<'a>(ctx: &TsCtx<'a>, mut expr: &'a Expression<'a>) -> &'a Expression<'a> {
    let mut seen = HashSet::new();
    let mut aliases = Vec::new();
    loop {
        expr = unwrapped(expr);
        let Expression::Identifier(id) = expr else {
            break;
        };
        if let Some(sym) = symbol(ctx, id)
            && let Some(&cached) = ctx.resolution.aliases.borrow().get(&sym)
        {
            expr = cached;
            break;
        }
        if !seen.insert(id.span.start) {
            return expr;
        }
        let Some(next) = initializer(ctx, id) else {
            break;
        };
        aliases.extend(symbol(ctx, id));
        expr = next;
    }
    ctx.resolution
        .aliases
        .borrow_mut()
        .extend(aliases.into_iter().map(|s| (s, expr)));
    expr
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
    id.name == name && ctx.semantic.is_reference_to_global_variable(id)
}

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
            (Self(ctx), oxc::ast::MemberExpressionKind::Computed(m)) => literal(ctx, &m.expression),
            (_, m) => m.static_property_name()?.as_str(),
        }
    }
}

pub(super) fn path<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<Vec<&'a str>> {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    let mut expr = expr;
    loop {
        expr = resolve(ctx, expr);
        if !seen.insert(expr.span()) {
            return None;
        }
        if let Some((object, key)) = receiver(ctx, expr) {
            parts.push(key);
            expr = object;
            continue;
        }
        if let Some(call) = call_expr(expr) {
            let id = call.callee.get_identifier_reference()?;
            if id.name != "require" || !ctx.semantic.is_reference_to_global_variable(id) {
                return None;
            }
            let [arg] = call.arguments.as_slice() else {
                return None;
            };
            parts.push(literal(ctx, arg.as_expression()?)?);
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
            if !global_object(id.name.as_str()) {
                parts.push(id.name.as_str());
            }
            break;
        }
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
    Some(parts)
}

pub(super) fn enclosing_functions<'ctx, 'a>(
    ctx: &'ctx TsCtx<'a>,
    id: oxc::syntax::node::NodeId,
) -> impl Iterator<Item = &'ctx oxc::semantic::AstNode<'a>> {
    ctx.semantic.nodes().ancestors(id).filter(|node| {
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
        AstKind::CallExpression(c) => Some(invoked(ctx, c)),
        _ => None,
    }
}

pub(super) fn outer<'a>(ctx: &TsCtx<'a>, id: oxc::syntax::node::NodeId) -> Option<AstKind<'a>> {
    ctx.semantic
        .nodes()
        .ancestors(id)
        .map(oxc::semantic::AstNode::kind)
        .find(|kind| {
            !matches!(
                kind,
                AstKind::ParenthesizedExpression(_)
                    | AstKind::ChainExpression(_)
                    | AstKind::TSAsExpression(_)
                    | AstKind::TSSatisfiesExpression(_)
                    | AstKind::TSInstantiationExpression(_)
                    | AstKind::TSNonNullExpression(_)
                    | AstKind::TSTypeAssertion(_)
            )
        })
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

pub(super) struct Invocation<'a> {
    pub(super) callee: &'a Expression<'a>,
    pub(super) path: Option<Vec<&'a str>>,

    pub(super) args: Vec<Option<&'a Expression<'a>>>,
}
impl<'a> Invocation<'a> {
    pub(super) fn arg(&self, i: usize) -> Option<&'a Expression<'a>> {
        self.args.get(i).copied().flatten()
    }
}
fn array_args<'a>(
    ctx: &TsCtx<'a>,
    arg: Option<&'a Expression<'a>>,
) -> Vec<Option<&'a Expression<'a>>> {
    if let Some(Expression::ArrayExpression(a)) = arg.map(|a| resolve(ctx, a)) {
        a.elements.iter().map(|e| e.as_expression()).collect()
    } else {
        Vec::new()
    }
}
pub(super) fn invoked<'a>(ctx: &TsCtx<'a>, call: &'a CallExpression<'a>) -> Rc<Invocation<'a>> {
    if let Some(hit) = ctx.resolution.calls.borrow().get(&call.span) {
        return Rc::clone(hit);
    }
    let mut callee = resolve(ctx, &call.callee);
    let mut args: Vec<_> = call.arguments.iter().map(Argument::as_expression).collect();
    let mut seen = HashSet::new();
    let path = loop {
        let parts = path(ctx, callee);

        if !seen.insert(callee.span()) {
            break parts;
        }
        if let Some(c) = call_expr(callee)
            && let Some(m) = resolve(ctx, &c.callee).get_member_expr()
            && MemberKeys(ctx).read(m) == Some("bind")
        {
            args.splice(..0, c.arguments.iter().skip(1).map(Argument::as_expression));
            callee = resolve(ctx, m.object());
            continue;
        }
        if parts.as_deref() == Some(&["Reflect", "apply"])
            && let Some(target) = args.first().copied().flatten()
        {
            args = array_args(ctx, args.get(2).copied().flatten());
            callee = resolve(ctx, target);
            continue;
        }
        let Some(m) = callee.get_member_expr() else {
            break parts;
        };
        match MemberKeys(ctx).read(m) {
            Some("call") => {
                args.drain(..args.len().min(1));
            }
            Some("apply") => args = array_args(ctx, args.get(1).copied().flatten()),
            _ => break parts,
        }
        callee = resolve(ctx, m.object());
    };
    let result = Rc::new(Invocation { callee, path, args });
    ctx.resolution
        .calls
        .borrow_mut()
        .insert(call.span, Rc::clone(&result));
    result
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

pub(super) struct Properties<'a> {
    pub(super) named: std::collections::BTreeMap<&'a str, &'a oxc::ast::ast::ObjectProperty<'a>>,
    pub(super) unknown: bool,
}
pub(super) fn properties<'a>(
    ctx: &TsCtx<'a>,
    initial: &'a Expression<'a>,
) -> Option<Properties<'a>> {
    enum Step<'a> {
        Value(&'a Expression<'a>),
        Property(&'a ObjectPropertyKind<'a>),
        Leave(u32),
    }
    let mut pending = vec![Step::Value(initial)];
    let mut active = HashSet::new();
    let mut result = Properties {
        named: std::collections::BTreeMap::new(),
        unknown: false,
    };
    if !matches!(resolve(ctx, initial), Expression::ObjectExpression(_)) {
        return None;
    }
    while let Some(step) = pending.pop() {
        match step {
            Step::Value(e) => {
                if let Expression::ObjectExpression(o) = resolve(ctx, e)
                    && active.insert(o.span.start)
                {
                    pending.push(Step::Leave(o.span.start));
                    pending.extend(o.properties.iter().rev().map(Step::Property));
                } else {
                    result.unknown = true;
                    result.named.clear();
                }
            }
            Step::Property(ObjectPropertyKind::ObjectProperty(p)) => {
                if let Some(name) = key(ctx, &p.key) {
                    result.named.insert(name, p);
                } else {
                    result.unknown = true;
                    result.named.clear();
                }
            }
            Step::Property(ObjectPropertyKind::SpreadProperty(s)) => {
                pending.push(Step::Value(&s.argument));
            }
            Step::Leave(start) => {
                active.remove(&start);
            }
        }
    }
    Some(result)
}

pub(super) fn receiver<'a>(
    ctx: &TsCtx<'a>,
    callee: &'a Expression<'a>,
) -> Option<(&'a Expression<'a>, &'a str)> {
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
    Some((
        v.init.as_ref()?,
        destructured(ctx, pattern, |b| b.name == id.name)?,
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
