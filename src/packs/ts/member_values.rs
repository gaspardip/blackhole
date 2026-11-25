use super::{
    declaration, has_with,
    strings::{Selector, SelectorTrie},
    symbol, transparent,
};
use crate::{engine::TsCtx, packs::ts::local_alias::is_local};
use oxc::{
    ast::{
        AstKind,
        ast::{
            Expression, IdentifierReference, NumericLiteral, ObjectPropertyKind, PropertyKey,
            PropertyKind,
        },
    },
    span::GetSpan,
    str::JSStr,
    syntax::{node::NodeId, symbol::SymbolId},
};
use std::{cell::RefCell, collections::HashMap};

pub(super) struct Fields<'a> {
    entries: HashMap<JSStr<'a>, &'a Expression<'a>>,
    selectors: Option<SelectorTrie<'a>>,
}
pub(super) type MemberValues<'a> = RefCell<HashMap<SymbolId, Option<Fields<'a>>>>;

pub(super) fn const_initializer<'a>(
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
    object: &'a Expression<'a>,
    selector: Selector<'a>,
) -> Option<&'a Expression<'a>> {
    let Expression::Identifier(id) = object.get_inner_expression() else {
        return None;
    };
    if inside_with(ctx, id.node_id.get()) {
        return None;
    }
    let sym = symbol(ctx, id)?;
    let mut cache = ctx.resolution.member_values.borrow_mut();
    let fields = cache.entry(sym).or_insert_with(|| fields(ctx, id, sym));
    let fields = fields.as_mut()?;
    selector.find(&fields.entries, &mut fields.selectors)
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
            match field.key.as_expression()?.get_inner_expression() {
                Expression::StringLiteral(key) => key.value,
                Expression::TemplateLiteral(key) => key.single_quasi()?,
                _ => return None,
            }
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
        fields.insert(key, &field.value);
    }
    (!fields.is_empty()).then_some(Fields {
        entries: fields,
        selectors: None,
    })
}

pub(super) fn plain_decimal_key<'a>(number: &NumericLiteral<'a>) -> Option<JSStr<'a>> {
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
    let mut parents = nodes.ancestor_kinds(id);
    let Some(member) = parents.find(|kind| !transparent(kind)) else {
        return false;
    };
    let object = match member {
        AstKind::StaticMemberExpression(m) => &m.object,
        AstKind::ComputedMemberExpression(m) => &m.object,
        AstKind::PrivateFieldExpression(m) => &m.object,
        AstKind::JSXMemberExpression(m) => return m.object.span() == child,
        _ => return false,
    };
    if object.get_inner_expression().span() != child {
        return false;
    }
    child = member.span();
    for parent in parents {
        match parent {
            AstKind::AssignmentExpression(a) => return a.left.span() != child,
            AstKind::UpdateExpression(u) => return u.argument.span() != child,
            AstKind::UnaryExpression(u) => {
                return !u.operator.is_delete() || u.argument.span() != child;
            }
            AstKind::ForInStatement(f) => return f.left.span() != child,
            AstKind::ForOfStatement(f) => return f.left.span() != child,
            AstKind::CallExpression(c) => return c.callee.span() != child,
            AstKind::TaggedTemplateExpression(t) => return t.tag.span() != child,
            AstKind::AssignmentTargetPropertyProperty(p) if p.binding.span() == child => {}
            AstKind::AssignmentTargetWithDefault(p) if p.binding.span() == child => {}
            AstKind::ObjectAssignmentTarget(_)
            | AstKind::ArrayAssignmentTarget(_)
            | AstKind::AssignmentTargetRest(_) => {}
            kind if transparent(&kind) => {}
            _ => return true,
        }
        child = parent.span();
    }
    true
}
