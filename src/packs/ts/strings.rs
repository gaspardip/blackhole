use super::{member_values, unwrapped};
use crate::engine::TsCtx;
use oxc::{
    ast::ast::{Expression, MemberExpression},
    str::JSStr,
    syntax::{node::NodeId, operator::BinaryOperator},
};
use std::{cell::RefCell, collections::HashMap, ptr};

const SHAPE: &[u8; 5] = b"shape";
const EDGE: usize = SHAPE.len() - 1;

pub(in crate::packs::ts) fn contains_shape(bytes: &[u8]) -> bool {
    bytes
        .windows(SHAPE.len())
        .any(|part| part.eq_ignore_ascii_case(SHAPE))
}

#[derive(Clone, Copy)]
enum Parts<'a> {
    Text(JSStr<'a>),
    Pair(&'a [Parts<'a>; 2]),
}

impl<'a> Parts<'a> {
    fn join(self, right: Self, ctx: &TsCtx<'a>) -> Self {
        match (self, right) {
            (Self::Text(s), _) if s.is_empty() => right,
            (_, Self::Text(s)) if s.is_empty() => self,
            _ => Self::Pair(ctx.arena.alloc([self, right])),
        }
    }

    fn identity(self) -> PartId<'a> {
        match self {
            Self::Text(text) => PartId::Text(ptr::from_ref(text.as_bytes())),
            Self::Pair(pair) => PartId::Pair(ptr::from_ref(pair)),
        }
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum PartId<'a> {
    Text(*const [u8]),
    Pair(*const [Parts<'a>; 2]),
}

pub(super) struct Selector<'a>(Parts<'a>);

impl<'a> Selector<'a> {
    fn literal(text: JSStr<'a>) -> Self {
        Self(Parts::Text(text))
    }

    pub(super) fn find(
        self,
        entries: &HashMap<JSStr<'a>, &'a Expression<'a>>,
        trie: &mut Option<SelectorTrie<'a>>,
    ) -> Option<&'a Expression<'a>> {
        match self.0 {
            Parts::Text(text) => entries.get(&text).copied(),
            parts @ Parts::Pair(_) => trie
                .get_or_insert_with(|| SelectorTrie::new(entries))
                .find(parts),
        }
    }
}

pub(super) struct SelectorTrie<'a> {
    edges: HashMap<(usize, u16), usize>,
    values: Vec<Option<&'a Expression<'a>>>,
    transitions: HashMap<(PartId<'a>, usize), Option<usize>>,
}

enum MatchStep<'a> {
    Visit(Parts<'a>),
    Remember(PartId<'a>, usize),
}

impl<'a> SelectorTrie<'a> {
    fn new(entries: &HashMap<JSStr<'a>, &'a Expression<'a>>) -> Self {
        let mut trie = Self {
            edges: HashMap::new(),
            values: vec![None],
            transitions: HashMap::new(),
        };
        for (&key, &value) in entries {
            let mut node = 0;
            // UTF-16 preserves matches across a surrogate pair split between parts.
            for unit in key.encode_utf16() {
                node = *trie.edges.entry((node, unit)).or_insert_with(|| {
                    trie.values.push(None);
                    trie.values.len() - 1
                });
            }
            trie.values[node] = Some(value);
        }
        trie
    }

    fn find(&mut self, parts: Parts<'a>) -> Option<&'a Expression<'a>> {
        let mut node = Some(0);
        let mut pending = vec![MatchStep::Visit(parts)];
        while let Some(step) = pending.pop() {
            match step {
                MatchStep::Remember(part, start) => {
                    self.transitions.insert((part, start), node);
                }
                MatchStep::Visit(parts) => {
                    let Some(start) = node else {
                        continue;
                    };
                    let key = (parts.identity(), start);
                    if let Some(&end) = self.transitions.get(&key) {
                        node = end;
                        continue;
                    }
                    match parts {
                        Parts::Text(text) => {
                            node = text.encode_utf16().try_fold(start, |node, unit| {
                                self.edges.get(&(node, unit)).copied()
                            });
                            self.transitions.insert(key, node);
                        }
                        Parts::Pair(&[left, right]) => pending.extend([
                            MatchStep::Remember(key.0, start),
                            MatchStep::Visit(right),
                            MatchStep::Visit(left),
                        ]),
                    }
                }
            }
        }
        self.values[node?]
    }
}

#[derive(Clone, Copy)]
struct ShapeMatch {
    prefix: [u8; EDGE],
    suffix: [u8; EDGE],
    found: bool,
}

impl ShapeMatch {
    fn leaf(bytes: &[u8]) -> Self {
        let edge = bytes.len().min(EDGE);
        let mut prefix = [0; EDGE];
        let mut suffix = [0; EDGE];
        prefix[..edge].copy_from_slice(&bytes[..edge]);
        suffix[EDGE - edge..].copy_from_slice(&bytes[bytes.len() - edge..]);
        Self {
            prefix,
            suffix,
            found: contains_shape(bytes),
        }
    }

    fn join(self, right: Self, left_len: usize, right_len: usize) -> Self {
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
            found: self.found || right.found || contains_shape(&boundary),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Fold<'a> {
    parts: Parts<'a>,
    len: usize,
    selector: bool,
    starts_low: bool,
    ends_high: bool,
    shape: ShapeMatch,
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
            shape: ShapeMatch::leaf(bytes),
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
            // Surrogate pairing cannot add or remove an ASCII match.
            shape: self.shape.join(right.shape, self.len, right.len),
        })
    }
}

pub(super) type StringCache<'a> = RefCell<HashMap<NodeId, Option<Fold<'a>>>>;

enum Step<'a> {
    Visit(&'a Expression<'a>),
    Copy(NodeId, NodeId, bool),
    Add(NodeId, NodeId, NodeId),
    Select(NodeId, &'a Expression<'a>, &'a Expression<'a>),
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
                                    Step::Select(id, &m.object, &m.expression),
                                    Step::Visit(&m.expression),
                                ]);
                            } else if let Some(key) = member.static_property_name()
                                && let Some(value) = member_values::value(
                                    ctx,
                                    member.object(),
                                    Selector::literal(key),
                                )
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
                let selector =
                    if let Expression::NumericLiteral(number) = key.get_inner_expression() {
                        member_values::plain_decimal_key(number).map(Selector::literal)
                    } else {
                        cache[&key.node_id()]
                            .filter(|key| key.selector)
                            .map(|key| Selector(key.parts))
                    };
                if let Some(value) =
                    selector.and_then(|selector| member_values::value(ctx, object, selector))
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

pub(in crate::packs::ts) fn static_string_contains_shape<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
) -> bool {
    fold(ctx, expr).is_some_and(|value| value.shape.found)
}
