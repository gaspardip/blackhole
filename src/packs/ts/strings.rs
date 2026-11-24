use super::{member_values, unwrapped};
use crate::engine::TsCtx;
use oxc::{
    ast::ast::{Expression, MemberExpression},
    str::{JSStr, JSStrBuilder},
    syntax::{node::NodeId, operator::BinaryOperator},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

#[derive(Clone, Copy)]
enum Parts<'a> {
    Text(JSStr<'a>),
    Pair(&'a Pair<'a>),
}

struct Pair<'a> {
    children: [Parts<'a>; 2],
    text: Cell<Option<JSStr<'a>>>,
}

impl<'a> Parts<'a> {
    fn join(self, right: Self, ctx: &TsCtx<'a>) -> Self {
        match (self, right) {
            (Self::Text(s), _) if s.is_empty() => right,
            (_, Self::Text(s)) if s.is_empty() => self,
            _ => Self::Pair(ctx.arena.alloc(Pair {
                children: [self, right],
                text: Cell::new(None),
            })),
        }
    }

    fn text(self, ctx: &TsCtx<'a>) -> JSStr<'a> {
        let root = match self {
            Self::Text(text) => return text,
            Self::Pair(root) => root,
        };
        if let Some(text) = root.text.get() {
            return text;
        }
        // Cache the shared children that many distinct root keys can reuse.
        for part in root.children {
            if let Self::Pair(pair) = part
                && pair.text.get().is_none()
            {
                pair.text.set(Some(part.flatten(ctx)));
            }
        }
        let text = self.flatten(ctx);
        root.text.set(Some(text));
        text
    }

    fn flatten(self, ctx: &TsCtx<'a>) -> JSStr<'a> {
        let mut text = JSStrBuilder::new_in(&ctx.arena);
        let mut pending = vec![self];
        while let Some(part) = pending.pop() {
            match part {
                Self::Text(leaf) => text.push_js_str(leaf),
                Self::Pair(pair) => {
                    if let Some(leaf) = pair.text.get() {
                        text.push_js_str(leaf);
                    } else {
                        let [left, right] = pair.children;
                        pending.extend([right, left]);
                    }
                }
            }
        }
        text.into_js_str()
    }
}

#[derive(Clone, Copy)]
pub(super) struct Fold<'a> {
    parts: Parts<'a>,
    len: usize,
    selector: bool,
    starts_low: bool,
    ends_high: bool,
}

impl<'a> Fold<'a> {
    fn leaf(text: JSStr<'a>) -> Self {
        let bytes = text.as_bytes();
        Self {
            parts: Parts::Text(text),
            len: text.len(),
            selector: true,
            starts_low: matches!(bytes, [0xed, 0xb0..=0xbf, _, ..]),
            ends_high: matches!(bytes, [.., 0xed, 0xa0..=0xaf, _]),
        }
    }

    fn add(self, right: Self, ctx: &TsCtx<'a>) -> Option<Self> {
        // Adjacent lone surrogates become one four-byte scalar.
        let joined_surrogates = self.ends_high && right.starts_low;
        let len = (self.len - 2 * usize::from(joined_surrogates)).checked_add(right.len)?;
        (len <= ctx.text.len()).then(|| Self {
            parts: self.parts.join(right.parts, ctx),
            len,
            selector: self.selector && right.selector,
            starts_low: self.starts_low || (self.len == 0 && right.starts_low),
            ends_high: right.ends_high || (right.len == 0 && self.ends_high),
        })
    }
}

pub(super) type StringCache<'a> = RefCell<HashMap<NodeId, Option<Fold<'a>>>>;

enum Step<'a> {
    Visit(&'a Expression<'a>),
    Copy(NodeId, NodeId, bool),
    Add(NodeId, NodeId, NodeId),
    Select(NodeId, &'a Expression<'a>, NodeId),
}

fn fold<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<Fold<'a>> {
    let mut cache = ctx.resolution.strings.borrow_mut();
    if let Some(&known) = cache.get(&expr.node_id()) {
        return known;
    }
    let mut pending = vec![Step::Visit(expr)];
    while let Some(step) = pending.pop() {
        let copy = |pending: &mut Vec<_>, id, next: &'a Expression<'a>, selector| {
            pending.extend([Step::Copy(id, next.node_id(), selector), Step::Visit(next)]);
        };
        let (id, result) = match step {
            Step::Visit(expr) => {
                let id = expr.node_id();
                if cache.contains_key(&id) {
                    continue;
                }
                // A dependency that returns to a pending node fails the cycle.
                cache.insert(id, None);
                let inner = unwrapped(expr);
                if inner.node_id() != id {
                    copy(&mut pending, id, inner, true);
                    continue;
                }
                let leaf = match expr {
                    Expression::StringLiteral(s) => Some(s.value),
                    Expression::TemplateLiteral(t) => t.single_quasi(),
                    Expression::Identifier(reference) => {
                        if let Some(next) = member_values::const_initializer(ctx, reference) {
                            copy(&mut pending, id, next, true);
                        }
                        continue;
                    }
                    Expression::BinaryExpression(b) if b.operator == BinaryOperator::Addition => {
                        pending.extend([
                            Step::Add(id, b.left.node_id(), b.right.node_id()),
                            Step::Visit(&b.right),
                            Step::Visit(&b.left),
                        ]);
                        continue;
                    }
                    _ => {
                        if let Some(member) = expr.get_member_expr() {
                            if let MemberExpression::ComputedMemberExpression(m) = member {
                                pending.extend([
                                    Step::Select(id, &m.object, m.expression.node_id()),
                                    Step::Visit(&m.expression),
                                ]);
                            } else if let Some(key) = member.static_property_name()
                                && let Some(value) = member_values::value(ctx, member.object(), key)
                            {
                                copy(&mut pending, id, value, false);
                            }
                        }
                        continue;
                    }
                };
                (
                    id,
                    leaf.filter(|leaf| leaf.len() <= ctx.text.len())
                        .map(Fold::leaf),
                )
            }
            Step::Copy(id, next, selector) => (
                id,
                cache[&next].map(|value| Fold {
                    selector: selector && value.selector,
                    ..value
                }),
            ),
            Step::Add(id, left, right) => {
                let value = cache[&left]
                    .zip(cache[&right])
                    .and_then(|(left, right)| left.add(right, ctx));
                (id, value)
            }
            Step::Select(id, object, key) => {
                if let Some(key) = cache[&key].filter(|key| key.selector)
                    && let Some(value) = member_values::value(ctx, object, key.parts.text(ctx))
                {
                    copy(&mut pending, id, value, false);
                }
                continue;
            }
        };
        cache.insert(id, result);
    }
    cache[&expr.node_id()]
}

pub(in crate::packs::ts) fn static_string<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
) -> Option<JSStr<'a>> {
    Some(fold(ctx, expr)?.parts.text(ctx))
}
