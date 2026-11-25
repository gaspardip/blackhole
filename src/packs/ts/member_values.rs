use super::{declaration, has_with, symbol, transparent, unwrapped};
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
use std::{cell::RefCell, collections::HashMap};

type Fields<'a> = HashMap<usize, &'a Expression<'a>>;

#[derive(Default)]
pub(super) struct MemberValues<'a> {
    owners: RefCell<HashMap<SymbolId, Option<Fields<'a>>>>,
    keys: RefCell<HashMap<JSStr<'a>, usize>>,
    selectors: RefCell<HashMap<NodeId, Option<usize>>>,
    target_ancestors: RefCell<HashMap<NodeId, bool>>,
}

impl<'a> MemberValues<'a> {
    fn key_id(&self, key: JSStr<'a>) -> usize {
        let mut keys = self.keys.borrow_mut();
        let next = keys.len();
        // Intern by value so reads through one const alias hash its string once.
        *keys.entry(key).or_insert(next)
    }

    fn selector(&self, ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<usize> {
        let expr = unwrapped(expr);
        let literal = match expr {
            Expression::NumericLiteral(number) => {
                return plain_decimal_key(number).map(|key| self.key_id(key));
            }
            Expression::Identifier(id) => const_initializer(ctx, id)?,
            _ => expr,
        };
        *self
            .selectors
            .borrow_mut()
            .entry(literal.node_id())
            .or_insert_with(|| literal_key(unwrapped(literal)).map(|key| self.key_id(key)))
    }
}

pub(in crate::packs::ts) fn selector_key<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
) -> Option<JSStr<'a>> {
    match unwrapped(expr) {
        Expression::Identifier(id) => literal_key(unwrapped(const_initializer(ctx, id)?)),
        expr => direct_selector_key(expr),
    }
}

fn direct_selector_key<'a>(expr: &Expression<'a>) -> Option<JSStr<'a>> {
    match expr {
        Expression::NumericLiteral(number) => plain_decimal_key(number),
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
        ctx.semantic.nodes().iter().any(|node| match node.kind() {
            AstKind::IdentifierReference(id) => id.name == "eval",
            AstKind::IdentifierName(id) => id.name == "eval",
            AstKind::BindingIdentifier(id) => id.name == "eval",
            AstKind::LabelIdentifier(id) => id.name == "eval",
            AstKind::PrivateIdentifier(id) => id.name == "eval",
            AstKind::JSXIdentifier(id) => id.name == "eval",
            AstKind::StringLiteral(literal) => literal.value == "eval",
            AstKind::TemplateLiteral(template) => {
                template.single_quasi().is_some_and(|value| value == "eval")
            }
            _ => false,
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
        for scope in scoping.scope_descendants_from_root() {
            let inherited = scoping
                .scope_parent_id(scope)
                .is_some_and(|parent| inside[parent.index()]);
            inside.push(inherited || scoping.scope_flags(scope).is_with());
        }
        inside
    });
    with_bodies[scope.index()]
}

pub(super) fn value<'a>(
    ctx: &TsCtx<'a>,
    member: &'a MemberExpression<'a>,
) -> Option<&'a Expression<'a>> {
    let Expression::Identifier(id) = member.object().get_inner_expression() else {
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
        _ => members.key_id(member.static_property_name()?),
    };
    fields.get(&key).copied()
}

fn fields<'a>(ctx: &TsCtx<'a>, id: &IdentifierReference<'_>, sym: SymbolId) -> Option<Fields<'a>> {
    let Expression::ObjectExpression(object) = const_initializer(ctx, id)?.get_inner_expression()
    else {
        return None;
    };
    let scope = ctx.semantic.scoping();
    if !is_local(ctx, sym)
        || !scope
            .get_resolved_references(sym)
            .all(|reference| readonly_member(ctx, reference.node_id()))
    {
        return None;
    }
    let mut fields = HashMap::new();
    for field in &object.properties {
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
                PropertyKey::NumericLiteral(number) => plain_decimal_key(number)?,
                _ => field.key.static_name()?.into_js_str_in(&ctx.arena),
            }
        };
        if key == "__proto__" {
            return None;
        }
        fields.insert(ctx.resolution.member_values.key_id(key), &field.value);
    }
    (!fields.is_empty()).then_some(fields)
}

fn plain_decimal_key<'a>(number: &NumericLiteral<'a>) -> Option<JSStr<'a>> {
    let raw = number.raw?;
    let plain_decimal = match raw.as_bytes() {
        [b'0'] => true,
        [b'1'..=b'9', rest @ ..] => rest.iter().all(u8::is_ascii_digit),
        _ => false,
    };
    (plain_decimal && (0.0..9_007_199_254_740_992.0).contains(&number.value)).then(|| raw.into())
}

fn readonly_member(ctx: &TsCtx<'_>, id: NodeId) -> bool {
    let nodes = ctx.semantic.nodes();
    let mut child = nodes.kind(id).span();
    let mut parents = nodes.ancestors(id);
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
    if object.get_inner_expression().span() != child {
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
