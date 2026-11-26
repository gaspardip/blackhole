use super::{declaration, has_with, inner, symbol, transparent, unwrapped};
use crate::{engine::TsCtx, packs::ts::local_alias::is_local};
use oxc::{
    ast::{
        AstKind,
        ast::{
            Expression, IdentifierReference, MemberExpression, NumericLiteral, ObjectPropertyKind,
            PropertyKey, PropertyKind,
        },
    },
    span::GetSpan,
    str::JSStr,
    syntax::{node::NodeId, symbol::SymbolId},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{BuildHasher, RandomState},
};

type Fields<'a> = HashMap<usize, &'a Expression<'a>>;

#[derive(Default)]
pub(super) struct MemberValues<'a> {
    owners: RefCell<HashMap<SymbolId, Option<Fields<'a>>>>,
    keys: RefCell<KeyIds<'a>>,
    selectors: RefCell<HashMap<NodeId, Option<usize>>>,
    target_ancestors: RefCell<HashMap<NodeId, bool>>,
}

#[derive(Default)]
struct KeyIds<'a> {
    hasher: RandomState,
    heads: HashMap<u64, usize>,
    entries: Vec<(JSStr<'a>, Option<usize>)>,
}

impl<'a> KeyIds<'a> {
    fn intern(&mut self, ctx: &TsCtx<'a>, key: JSStr<'a>) -> Option<usize> {
        if !ctx.take_string_work(key.len()) {
            return None;
        }
        // Retain only fixed-size hashes in the table so growth cannot reread the keys.
        let hash = self.hasher.hash_one(key);
        let head = self.heads.get(&hash).copied();
        let mut candidate = head;
        while let Some(id) = candidate {
            if !ctx.take_work(1) {
                return None;
            }
            let (stored, next) = self.entries[id];
            if super::same_string_bytes(ctx, stored.as_bytes(), key.as_bytes()) {
                return Some(id);
            }
            candidate = next;
        }
        if !ctx.resolution.has_work() {
            return None;
        }
        let id = self.entries.len();
        self.entries.push((key, head));
        self.heads.insert(hash, id);
        Some(id)
    }
}

impl<'a> MemberValues<'a> {
    fn key_id(&self, ctx: &TsCtx<'a>, key: JSStr<'a>) -> Option<usize> {
        self.keys.borrow_mut().intern(ctx, key)
    }

    fn selector(&self, ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<usize> {
        let expr = unwrapped(ctx, expr);
        let literal = match expr {
            Expression::Identifier(id) => const_initializer(ctx, id)?,
            _ => expr,
        };
        *self
            .selectors
            .borrow_mut()
            .entry(literal.node_id())
            .or_insert_with(|| selector_key(ctx, expr).and_then(|key| self.key_id(ctx, key)))
    }
}

pub(super) fn selector_key<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<JSStr<'a>> {
    match unwrapped(ctx, expr) {
        Expression::Identifier(id) => literal_key(unwrapped(ctx, const_initializer(ctx, id)?)),
        expr => literal_member_key(ctx, expr),
    }
}

fn literal_member_key<'a>(ctx: &TsCtx<'_>, expr: &Expression<'a>) -> Option<JSStr<'a>> {
    match expr {
        Expression::NumericLiteral(number) => plain_decimal_key(ctx, number),
        literal => literal_key(literal),
    }
}

fn literal_key<'a>(expr: &Expression<'a>) -> Option<JSStr<'a>> {
    match expr {
        Expression::StringLiteral(s) => Some(s.value),
        Expression::TemplateLiteral(t) => t.single_quasi(),
        _ => None,
    }
}

pub(in crate::packs::ts) fn const_initializer<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
) -> Option<&'a Expression<'a>> {
    ctx.record_work(1);
    if blocks_identifier_resolution(ctx, id.node_id.get()) {
        return None;
    }
    let symbol = symbol(ctx, id)?;
    if !ctx
        .semantic
        .scoping()
        .symbol_flags(symbol)
        .is_const_variable()
    {
        return None;
    }
    let AstKind::VariableDeclarator(v) = declaration(ctx, id)? else {
        return None;
    };
    v.id.get_binding_identifier()?;
    v.init.as_ref()
}

fn blocks_identifier_resolution(ctx: &TsCtx<'_>, id: NodeId) -> bool {
    has_eval_mention(ctx) || inside_with(ctx, id)
}

fn has_eval_mention(ctx: &TsCtx<'_>) -> bool {
    *ctx.resolution.eval_mention.get_or_init(|| {
        ctx.nodes().any(|node| {
            let name = match node.kind() {
                AstKind::IdentifierReference(id) => Some(id.name.as_js_str()),
                AstKind::IdentifierName(id) => Some(id.name.as_js_str()),
                AstKind::BindingIdentifier(id) => Some(id.name.as_js_str()),
                AstKind::LabelIdentifier(id) => Some(id.name.as_js_str()),
                AstKind::PrivateIdentifier(id) => Some(id.name.as_js_str()),
                AstKind::JSXIdentifier(id) => Some(id.name.into()),
                AstKind::StringLiteral(literal) => Some(literal.value),
                AstKind::TemplateLiteral(template) => template.single_quasi(),
                _ => None,
            };
            name.is_some_and(|name| super::same_string_bytes(ctx, name.as_bytes(), b"eval"))
        })
    })
}

fn inside_with(ctx: &TsCtx<'_>, id: NodeId) -> bool {
    if !has_with(ctx) {
        return false;
    }
    let scope = ctx.semantic.nodes().get_node(id).scope_id();
    let with_bodies = ctx.resolution.with_bodies.get_or_init(|| {
        let scoping = ctx.semantic.scoping();
        let mut inside = Vec::with_capacity(scoping.scopes_len());
        // Oxc assigns scope IDs in parent-first order.
        for scope in ctx.counted(scoping.scope_descendants_from_root()) {
            let inherited = scoping
                .scope_parent_id(scope)
                .is_some_and(|parent| inside[parent.index()]);
            inside.push(inherited || scoping.scope_flags(scope).is_with());
        }
        inside
    });
    with_bodies[scope.index()]
}

pub(in crate::packs::ts) fn value<'a>(
    ctx: &TsCtx<'a>,
    member: &'a MemberExpression<'a>,
) -> Option<&'a Expression<'a>> {
    let Expression::Identifier(id) = inner(ctx, member.object()) else {
        return None;
    };
    if inside_with(ctx, id.node_id.get()) {
        return None;
    }
    let sym = symbol(ctx, id)?;
    let members = &ctx.resolution.member_values;
    let mut cache = members.owners.borrow_mut();
    let fields = cache
        .entry(sym)
        .or_insert_with(|| fields(ctx, id, sym))
        .as_ref()?;
    let key = match member {
        MemberExpression::ComputedMemberExpression(m) => members.selector(ctx, &m.expression)?,
        _ => members.key_id(ctx, member.static_property_name()?)?,
    };
    fields.get(&key).copied()
}

fn fields<'a>(ctx: &TsCtx<'a>, id: &IdentifierReference<'_>, sym: SymbolId) -> Option<Fields<'a>> {
    let Expression::ObjectExpression(object) = inner(ctx, const_initializer(ctx, id)?) else {
        return None;
    };
    let scope = ctx.semantic.scoping();
    if !is_local(ctx, sym)
        || !scope
            .get_resolved_references(sym)
            .all(|reference| ctx.take_work(1) && readonly_member(ctx, reference.node_id()))
    {
        return None;
    }
    let mut fields = HashMap::new();
    for field in &object.properties {
        if !ctx.take_work(1) {
            return None;
        }
        let ObjectPropertyKind::ObjectProperty(field) = field else {
            return None;
        };
        if field.kind != PropertyKind::Init {
            return None;
        }
        let key = if field.computed {
            literal_key(field.key.as_expression()?.get_inner_expression())?
        } else {
            match &field.key {
                PropertyKey::BigIntLiteral(_) => return None,
                PropertyKey::NumericLiteral(number) => plain_decimal_key(ctx, number)?,
                PropertyKey::RegExpLiteral(regex) => {
                    if !ctx.take_string_work(regex.span.size() as usize) {
                        return None;
                    }
                    let name = field.key.static_name()?;
                    if !ctx.take_string_work(name.as_js_str().len()) {
                        return None;
                    }
                    name.into_js_str_in(&ctx.arena)
                }
                _ => field.key.static_name()?.into_js_str_in(&ctx.arena),
            }
        };
        if super::same_string_bytes(ctx, key.as_bytes(), b"__proto__") {
            return None;
        }
        fields.insert(ctx.resolution.member_values.key_id(ctx, key)?, &field.value);
    }
    (!fields.is_empty()).then_some(fields)
}

fn plain_decimal_key<'a>(ctx: &TsCtx<'_>, number: &NumericLiteral<'a>) -> Option<JSStr<'a>> {
    let raw = number.raw?;
    if raw.len() > 16
        || !(0.0..9_007_199_254_740_992.0).contains(&number.value)
        || !ctx.take_string_work(raw.len())
    {
        return None;
    }
    let plain_decimal = match raw.as_bytes() {
        [b'0'] => true,
        [b'1'..=b'9', rest @ ..] => rest.iter().all(u8::is_ascii_digit),
        _ => false,
    };
    plain_decimal.then(|| raw.into())
}

fn readonly_member(ctx: &TsCtx<'_>, id: NodeId) -> bool {
    let nodes = ctx.semantic.nodes();
    let mut child = nodes.kind(id).span();
    let mut parents = ctx.bounded_ancestors(id);
    let Some(member) = parents.find(|node| !transparent(&node.kind())) else {
        return false;
    };
    let object = match member.kind() {
        AstKind::StaticMemberExpression(m) => &m.object,
        AstKind::ComputedMemberExpression(m) => &m.object,
        AstKind::PrivateFieldExpression(m) => &m.object,
        AstKind::JSXMemberExpression(m) => return m.object.span() == child,
        _ => return false,
    };
    if inner(ctx, object).span() != child {
        return false;
    }
    child = member.kind().span();
    let mut cache = ctx.resolution.member_values.target_ancestors.borrow_mut();
    let mut path = Vec::new();
    let readonly = loop {
        let Some(parent) = parents.next() else {
            break true;
        };
        let kind = parent.kind();
        match kind {
            AstKind::AssignmentExpression(a) => break a.left.span() != child,
            AstKind::UpdateExpression(u) => break u.argument.span() != child,
            AstKind::UnaryExpression(u) => {
                break !u.operator.is_delete() || u.argument.span() != child;
            }
            AstKind::ForInStatement(f) => break f.left.span() != child,
            AstKind::ForOfStatement(f) => break f.left.span() != child,
            AstKind::CallExpression(c) => break c.callee.span() != child,
            AstKind::TaggedTemplateExpression(t) => break t.tag.span() != child,
            AstKind::AssignmentTargetPropertyProperty(p) if p.binding.span() == child => {}
            AstKind::AssignmentTargetWithDefault(p) if p.binding.span() == child => {}
            AstKind::ObjectAssignmentTarget(_)
            | AstKind::ArrayAssignmentTarget(_)
            | AstKind::AssignmentTargetRest(_) => {}
            kind if transparent(&kind) => {}
            _ => break true,
        }
        if let Some(&readonly) = cache.get(&parent.id()) {
            break readonly;
        }
        path.push(parent.id());
        child = kind.span();
    };
    cache.extend(path.into_iter().map(|node| (node, readonly)));
    readonly
}
