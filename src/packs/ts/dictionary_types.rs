use super::function_types::bindings::{symbol, type_declaration};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{TSSignature, TSType, TSTypeName, TSTypeOperatorOperator, TSTypeReference},
    },
    syntax::{node::NodeId, symbol::SymbolId},
};
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Class {
    None,
    Broad,
    Dictionary,
    Any,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Query {
    UnsafeDictionary,
    ObjectInput,
    UnknownAlias,
    UnknownReturn,
    Target,
    AliasTarget,
    Key,
    Value,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Term {
    node: NodeId,
    env: usize,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Goal {
    term: Term,
    query: Query,
}
impl Goal {
    fn child(self, ty: &TSType<'_>, query: Query) -> Self {
        Self {
            term: Term {
                node: ty.node_id(),
                env: self.term.env,
            },
            query,
        }
    }
}
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct Binding {
    outer: usize,
    symbol: SymbolId,
    value: Term,
}

#[derive(Default)]
struct Environments {
    frames: Vec<Binding>,
    ids: HashMap<Binding, usize>,
}
impl Environments {
    fn bind(&mut self, b: Binding) -> usize {
        *self.ids.entry(b).or_insert_with(|| {
            self.frames.push(b);
            self.frames.len()
        })
    }
    fn lookup(&self, mut env: usize, symbol: SymbolId) -> Option<Term> {
        while env != 0 {
            let binding = self.frames[env - 1];
            if binding.symbol == symbol {
                return Some(binding.value);
            }
            env = binding.outer;
        }
        None
    }
}
#[derive(Clone, Copy)]
enum Combine {
    First,
    Any,
    All,
    Dictionary,
    OnlyDictionary,
}
impl Combine {
    fn apply(self, values: &[Class]) -> Class {
        let first = values
            .iter()
            .copied()
            .find(|v| *v != Class::None)
            .unwrap_or(Class::None);
        match self {
            Self::First => first,
            Self::Any if first != Class::None => Class::Broad,
            Self::Dictionary if first != Class::None => Class::Dictionary,
            Self::OnlyDictionary if first == Class::Dictionary => first,
            Self::All if values.contains(&Class::Any) => Class::Any,
            Self::All if !values.is_empty() && !values.contains(&Class::None) => Class::Broad,
            _ => Class::None,
        }
    }
}

enum Plan {
    Done(Class),
    Children(Combine, Children),
    Alias(NodeId, Combine, Goal),
}
enum Children {
    One(Goal),
    Many(Vec<Goal>),
}
enum Work {
    Visit(Goal),
    Finish {
        goal: Goal,
        combine: Combine,
        start: usize,
        outer_cutoff: usize,
        alias: Option<NodeId>,
    },
}

#[derive(Default)]
struct Solver {
    envs: Environments,
    memo: HashMap<Goal, Class>,
    scratch: Scratch,
}
impl Solver {
    fn solve(&mut self, ctx: &TsCtx<'_>, goal: Goal) -> Class {
        self.scratch
            .evaluate(&mut self.memo, goal, |g| self.envs.plan(ctx, g))
    }
}

#[derive(Default)]
struct Scratch {
    work: Vec<Work>,
    values: Vec<Class>,
    active: HashMap<Goal, usize>,
    aliases: HashMap<NodeId, usize>,
}
impl Scratch {
    fn evaluate(
        &mut self,
        memo: &mut HashMap<Goal, Class>,
        goal: Goal,
        mut plan: impl FnMut(Goal) -> Plan,
    ) -> Class {
        if let Some(&cached) = memo.get(&goal) {
            return cached;
        }
        let Self {
            work,
            values,
            active,
            aliases,
        } = self;
        work.push(Work::Visit(goal));
        let mut cutoff = usize::MAX;
        while let Some(task) = work.pop() {
            match task {
                Work::Visit(g) => {
                    if let Some(value) = memo.get(&g) {
                        values.push(*value);
                        continue;
                    }
                    if let Some(&level) = active.get(&g) {
                        cutoff = cutoff.min(level);
                        values.push(Class::None);
                        continue;
                    }
                    let level = active.len();
                    let (combine, children, alias) = match plan(g) {
                        Plan::Done(c) => {
                            values.push(c);
                            memo.insert(g, c);
                            continue;
                        }
                        Plan::Children(c, kids) => (c, kids, None),
                        Plan::Alias(id, c, child) => {
                            if let Some(&ancestor) = aliases.get(&id) {
                                cutoff = cutoff.min(ancestor);
                                values.push(Class::None);
                                continue;
                            }
                            aliases.insert(id, level);
                            (c, Children::One(child), Some(id))
                        }
                    };
                    active.insert(g, level);
                    work.push(Work::Finish {
                        goal: g,
                        combine,
                        start: values.len(),
                        outer_cutoff: std::mem::replace(&mut cutoff, usize::MAX),
                        alias,
                    });
                    match children {
                        Children::One(child) => work.push(Work::Visit(child)),
                        Children::Many(kids) => {
                            work.extend(kids.into_iter().rev().map(Work::Visit));
                        }
                    }
                }
                Work::Finish {
                    goal,
                    combine,
                    start,
                    outer_cutoff,
                    alias,
                } => {
                    let value = combine.apply(&values[start..]);
                    values.truncate(start);
                    values.push(value);
                    let level = active.remove(&goal).unwrap();
                    if let Some(id) = alias {
                        aliases.remove(&id);
                    }
                    // A result that used an enclosing cycle cut is only valid on this path.
                    if cutoff >= level {
                        cutoff = usize::MAX;
                        memo.insert(goal, value);
                    }
                    cutoff = cutoff.min(outer_cutoff);
                }
            }
        }
        values.pop().unwrap()
    }
}
impl Plan {
    fn child(combine: Combine, goal: Goal) -> Self {
        Self::Children(combine, Children::One(goal))
    }
    fn many(combine: Combine, goals: Vec<Goal>) -> Self {
        Self::Children(combine, Children::Many(goals))
    }
}
impl Environments {
    fn reference(&mut self, ctx: &TsCtx<'_>, g: Goal, r: &TSTypeReference<'_>) -> Plan {
        let TSTypeName::IdentifierReference(name) = &r.type_name else {
            return Plan::Done(Class::None);
        };
        let args = r
            .type_arguments
            .as_ref()
            .map_or(&[][..], |a| a.params.as_slice());
        let scopes = ctx.semantic.scoping();
        if let Some(sym) = symbol(ctx, name) {
            if scopes.symbol_flags(sym).is_type_parameter()
                && let Some(term) = self.lookup(g.term.env, sym)
            {
                return Plan::child(Combine::First, Goal { term, ..g });
            }
            let Some(id) = type_declaration(ctx, sym) else {
                return Plan::Done(Class::None);
            };
            match ctx.semantic.nodes().kind(id) {
                AstKind::TSInterfaceDeclaration(i)
                    if g.query == Query::Value
                        && i.extends.is_empty()
                        && empty(&i.body.body)
                        && scopes
                            .symbol_declarations(sym)
                            .filter(|d| {
                                matches!(
                                    ctx.semantic.nodes().kind(*d),
                                    AstKind::TSInterfaceDeclaration(_)
                                )
                            })
                            .count()
                            == 1 =>
                {
                    return Plan::Done(Class::Broad);
                }
                AstKind::TSTypeAliasDeclaration(a) => {
                    let mut env = 0;
                    if let Some(params) = &a.type_parameters {
                        for (index, p) in params.params.iter().enumerate() {
                            let (arg, arg_env) = if let Some(arg) = args.get(index) {
                                (arg, g.term.env)
                            } else if let Some(arg) = &p.default {
                                (arg, env)
                            } else {
                                return Plan::Done(Class::None);
                            };
                            let value = self.argument(ctx, arg, arg_env);
                            env = self.bind(Binding {
                                outer: env,
                                symbol: p.name.symbol_id.get().unwrap(),
                                value,
                            });
                        }
                    }
                    let query = if g.query == Query::Target {
                        Query::AliasTarget
                    } else {
                        g.query
                    };
                    let combine = if g.query == Query::Target && a.type_parameters.is_some() {
                        Combine::OnlyDictionary
                    } else {
                        Combine::First
                    };
                    return Plan::Alias(
                        id,
                        combine,
                        Goal {
                            term: Term {
                                node: a.type_annotation.node_id(),
                                env,
                            },
                            query,
                        },
                    );
                }
                _ => return Plan::Done(Class::None),
            }
        }
        let (index, query, combine) = match (name.name.as_str(), g.query) {
            (
                "Readonly" | "NonNullable" | "Partial" | "Required",
                Query::Target | Query::AliasTarget | Query::Value | Query::UnsafeDictionary,
            )
            | ("Promise" | "PromiseLike", Query::UnknownReturn)
            | ("Pick" | "Omit", Query::UnsafeDictionary) => (0, g.query, Combine::First),
            ("Record", Query::UnsafeDictionary) => (1, Query::Value, Combine::First),
            ("Record", Query::Target | Query::AliasTarget) if args.is_empty() => {
                return Plan::Done(Class::Dictionary);
            }
            ("Record", Query::Target | Query::AliasTarget) => (0, Query::Key, Combine::Dictionary),
            ("PropertyKey", Query::Key) => return Plan::Done(Class::Broad),
            _ => return Plan::Done(Class::None),
        };
        args.get(index).map_or(Plan::Done(Class::None), |t| {
            Plan::child(combine, g.child(t, query))
        })
    }
    fn argument(&mut self, ctx: &TsCtx<'_>, ty: &TSType<'_>, env: usize) -> Term {
        if let TSType::TSTypeReference(r) = ty.without_parenthesized()
            && let TSTypeName::IdentifierReference(i) = &r.type_name
            && let Some(sym) = symbol(ctx, i)
            && let Some(value) = self.lookup(env, sym)
        {
            return value;
        }
        Term {
            node: ty.node_id(),
            env,
        }
    }
    fn plan(&mut self, ctx: &TsCtx<'_>, g: Goal) -> Plan {
        let q = g.query;
        match (ctx.semantic.nodes().kind(g.term.node), q) {
            (AstKind::TSTypeReference(r), _) => self.reference(ctx, g, r),
            (AstKind::TSParenthesizedType(p), _) => {
                Plan::child(Combine::First, g.child(&p.type_annotation, q))
            }
            (AstKind::TSTypeOperator(p), _) if p.operator == TSTypeOperatorOperator::Readonly => {
                Plan::child(Combine::First, g.child(&p.type_annotation, q))
            }
            (AstKind::TSUnknownKeyword(_), Query::UnknownReturn | Query::UnknownAlias)
            | (AstKind::TSObjectKeyword(_), Query::ObjectInput)
            | (
                AstKind::TSObjectKeyword(_) | AstKind::TSUnknownKeyword(_),
                Query::Value | Query::Target | Query::AliasTarget,
            )
            | (
                AstKind::TSStringKeyword(_)
                | AstKind::TSNumberKeyword(_)
                | AstKind::TSSymbolKeyword(_),
                Query::Key,
            ) => Plan::Done(Class::Broad),
            (AstKind::TSAnyKeyword(_), Query::Value) => Plan::Done(Class::Any),
            (
                AstKind::TSUnionType(n),
                Query::Value
                | Query::Key
                | Query::UnknownReturn
                | Query::UnknownAlias
                | Query::ObjectInput,
            ) => Plan::many(
                Combine::Any,
                n.types.iter().map(|ty| g.child(ty, q)).collect(),
            ),
            (AstKind::TSIntersectionType(n), Query::Value) => Plan::many(
                Combine::All,
                n.types.iter().map(|ty| g.child(ty, q)).collect(),
            ),
            (AstKind::TSTypeLiteral(n), Query::Value) if empty(&n.members) => {
                Plan::Done(Class::Broad)
            }
            (AstKind::TSTypeLiteral(n), Query::UnsafeDictionary) => Plan::many(
                Combine::First,
                n.members
                    .iter()
                    .filter_map(|m| {
                        let TSSignature::TSIndexSignature(i) = m else {
                            return None;
                        };
                        Some(g.child(&i.type_annotation.type_annotation, Query::Value))
                    })
                    .collect(),
            ),
            (AstKind::TSTypeLiteral(n), Query::Target | Query::AliasTarget)
                if n.members
                    .iter()
                    .any(|m| matches!(m, TSSignature::TSIndexSignature(_))) =>
            {
                Plan::Done(Class::Dictionary)
            }
            (AstKind::TSTypeLiteral(n), Query::Target) if !n.members.is_empty() => {
                Plan::Done(Class::Broad)
            }
            (AstKind::TSMappedType(_), Query::Target) => Plan::Done(Class::Dictionary),
            (AstKind::TSMappedType(n), Query::AliasTarget) => {
                Plan::child(Combine::Dictionary, g.child(&n.constraint, Query::Key))
            }
            (AstKind::TSMappedType(n), Query::UnsafeDictionary) => n
                .type_annotation
                .as_ref()
                .map_or(Plan::Done(Class::None), |ty| {
                    Plan::child(Combine::First, g.child(ty, Query::Value))
                }),
            _ => Plan::Done(Class::None),
        }
    }
}

fn empty(members: &[TSSignature<'_>]) -> bool {
    members.iter().all(|m| {
        let TSSignature::TSPropertySignature(p) = m else {
            return false;
        };
        p.optional
            && p.type_annotation.as_ref().is_some_and(|a| {
                let mut ty = a.type_annotation.without_parenthesized();
                while let TSType::TSTypeOperatorType(op) = ty {
                    if op.operator != TSTypeOperatorOperator::Readonly {
                        return false;
                    }
                    ty = op.type_annotation.without_parenthesized();
                }
                matches!(ty, TSType::TSNeverKeyword(_))
            })
    })
}

#[derive(Default)]
pub(super) struct Resolution(RefCell<Solver>);
impl Resolution {
    fn query(&self, ctx: &TsCtx<'_>, node: NodeId, query: Query) -> Class {
        self.0.borrow_mut().solve(
            ctx,
            Goal {
                term: Term { node, env: 0 },
                query,
            },
        )
    }
    pub(super) fn unsafe_dictionary(&self, ctx: &TsCtx<'_>, kind: AstKind<'_>) -> bool {
        matches!(
            kind,
            AstKind::TSTypeReference(_) | AstKind::TSTypeLiteral(_) | AstKind::TSMappedType(_)
        ) && self.query(ctx, kind.node_id(), Query::UnsafeDictionary) != Class::None
    }
    pub(super) fn target(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Class {
        self.query(ctx, ty.node_id(), Query::Target)
    }
    pub(super) fn informative(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> bool {
        self.query(ctx, ty.node_id(), Query::Value) == Class::None
    }
    pub(super) fn object_input(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> bool {
        self.query(ctx, ty.node_id(), Query::ObjectInput) != Class::None
    }
    pub(super) fn unknown_alias(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> bool {
        self.query(ctx, ty.node_id(), Query::UnknownAlias) != Class::None
    }
    pub(super) fn unknown_return(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> bool {
        self.query(ctx, ty.node_id(), Query::UnknownReturn) != Class::None
    }
}

#[cfg(test)]
mod tests {
    use super::{Class, Combine, Goal, Plan, Query, Scratch, Term};
    use oxc::syntax::node::NodeId;
    use std::collections::HashMap;

    #[test]
    fn each_shared_goal_is_expanded_once_on_a_deep_work_stack() {
        let depth = 10_000;
        let goal = |i| Goal {
            term: Term {
                node: NodeId::new(i),
                env: 0,
            },
            query: Query::UnknownAlias,
        };
        let mut calls = 0;
        let mut scratch = Scratch::default();
        let mut memo = HashMap::new();
        let mut plan = |g: Goal| {
            calls += 1;
            assert!(calls <= depth + 1, "a shared goal was expanded twice");
            let i = g.term.node.index();
            if i == 0 {
                Plan::Done(Class::Broad)
            } else if i.is_multiple_of(2) {
                Plan::many(Combine::Any, vec![goal(i - 1), goal(i - 1)])
            } else {
                Plan::Alias(g.term.node, Combine::First, goal(i - 1))
            }
        };
        assert!(scratch.evaluate(&mut memo, goal(depth), &mut plan) == Class::Broad);
        for i in 0..depth {
            assert!(scratch.evaluate(&mut memo, goal(i), &mut plan) == Class::Broad);
        }
        assert_eq!(calls, depth + 1);
    }
}
