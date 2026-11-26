use super::{member_values, unwrapped};
use crate::engine::TsCtx;
use oxc::{
    ast::ast::Expression,
    str::JSStr,
    syntax::{node::NodeId, operator::BinaryOperator},
};
use std::{cell::RefCell, collections::HashMap};

const SHAPE: &[u8; 5] = b"shape";
const EDGE: usize = SHAPE.len() - 1;

pub(in crate::packs::ts) fn contains_shape(ctx: &TsCtx<'_>, bytes: &[u8]) -> bool {
    let found = bytes
        .windows(SHAPE.len())
        .position(|part| part.eq_ignore_ascii_case(SHAPE));
    ctx.record_string_work(found.map_or(bytes.len().saturating_sub(EDGE), |index| index + 1));
    found.is_some()
}

#[derive(Clone, Copy)]
struct ShapeMatch {
    prefix: [u8; EDGE],
    suffix: [u8; EDGE],
    found: bool,
}

impl ShapeMatch {
    fn leaf(ctx: &TsCtx<'_>, bytes: &[u8]) -> Self {
        let edge = bytes.len().min(EDGE);
        let mut prefix = [0; EDGE];
        let mut suffix = [0; EDGE];
        prefix[..edge].copy_from_slice(&bytes[..edge]);
        suffix[EDGE - edge..].copy_from_slice(&bytes[bytes.len() - edge..]);
        Self {
            prefix,
            suffix,
            found: contains_shape(ctx, bytes),
        }
    }

    fn join(self, ctx: &TsCtx<'_>, right: Self, left_len: usize, right_len: usize) -> Self {
        let mut prefix = self.prefix;
        let left_edge = left_len.min(EDGE);
        prefix[left_edge..].copy_from_slice(&right.prefix[..EDGE - left_edge]);
        let mut suffix = right.suffix;
        let right_edge = right_len.min(EDGE);
        suffix[..EDGE - right_edge].copy_from_slice(&self.suffix[right_edge..]);
        let mut boundary = [0; 2 * EDGE];
        boundary[..EDGE].copy_from_slice(&self.suffix);
        boundary[EDGE..].copy_from_slice(&right.prefix);
        Self {
            prefix,
            suffix,
            found: self.found || right.found || contains_shape(ctx, &boundary),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Fold {
    len: usize,
    starts_low: bool,
    ends_high: bool,
    shape: ShapeMatch,
}

impl Fold {
    fn leaf(ctx: &TsCtx<'_>, text: JSStr<'_>) -> Self {
        let bytes = text.as_bytes();
        Self {
            len: text.len(),
            starts_low: matches!(bytes, [0xed, 0xb0..=0xbf, _, ..]),
            ends_high: matches!(bytes, [.., 0xed, 0xa0..=0xaf, _]),
            shape: ShapeMatch::leaf(ctx, bytes),
        }
    }

    fn add(self, right: Self, ctx: &TsCtx<'_>) -> Option<Self> {
        // Adjacent lone surrogates become one four-byte scalar.
        let joined_surrogates = self.ends_high && right.starts_low;
        let len = (self.len - 2 * usize::from(joined_surrogates)).checked_add(right.len)?;
        (len <= ctx.text.len()).then(|| Self {
            len,
            starts_low: self.starts_low || (self.len == 0 && right.starts_low),
            ends_high: right.ends_high || (right.len == 0 && self.ends_high),
            // Surrogate pairing cannot add or remove an ASCII match.
            shape: self.shape.join(ctx, right.shape, self.len, right.len),
        })
    }
}

pub(super) type StringCache = RefCell<HashMap<NodeId, Option<Fold>>>;

enum Step<'a> {
    Visit(&'a Expression<'a>),
    Copy(NodeId, NodeId),
    Add(NodeId, NodeId, NodeId),
}

fn fold<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<Fold> {
    ctx.record_work(1);
    let mut cache = ctx.resolution.strings.borrow_mut();
    if let Some(&known) = cache.get(&expr.node_id()) {
        return known;
    }
    let mut pending = vec![Step::Visit(expr)];
    while let Some(step) = pending.pop() {
        ctx.record_work(1);
        let copy = |pending: &mut Vec<_>, id, next: &'a Expression<'a>| {
            pending.extend([Step::Copy(id, next.node_id()), Step::Visit(next)]);
        };
        let (id, result) = match step {
            Step::Visit(expr) => {
                let id = expr.node_id();
                if cache.contains_key(&id) {
                    continue;
                }
                // A dependency that returns to a pending node fails the cycle.
                cache.insert(id, None);
                let inner = unwrapped(ctx, expr);
                if inner.node_id() != id {
                    copy(&mut pending, id, inner);
                    continue;
                }
                let leaf = match expr {
                    Expression::StringLiteral(s) => Some(s.value),
                    Expression::TemplateLiteral(t) => t.single_quasi(),
                    Expression::Identifier(reference) => {
                        if let Some(next) = member_values::const_initializer(ctx, reference) {
                            copy(&mut pending, id, next);
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
                        if let Some(member) = expr.get_member_expr()
                            && let Some(value) = member_values::value(ctx, member)
                        {
                            copy(&mut pending, id, value);
                        }
                        continue;
                    }
                };
                (
                    id,
                    leaf.filter(|leaf| leaf.len() <= ctx.text.len())
                        .map(|leaf| Fold::leaf(ctx, leaf)),
                )
            }
            Step::Copy(id, next) => (id, cache[&next]),
            Step::Add(id, left, right) => {
                let value = cache[&left]
                    .zip(cache[&right])
                    .and_then(|(left, right)| left.add(right, ctx));
                (id, value)
            }
        };
        cache.insert(id, result);
    }
    cache[&expr.node_id()]
}

pub(in crate::packs::ts) fn static_string_contains_shape<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
) -> bool {
    fold(ctx, expr).is_some_and(|value| value.shape.found)
}
