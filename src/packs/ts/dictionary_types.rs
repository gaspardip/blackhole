use super::function_types::bindings::{is_type_declaration, symbol};
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
    place: usize,
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
                ..self.term
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
    places: Vec<(usize, NodeId)>,
    place_ids: HashMap<(usize, NodeId), usize>,
}
impl Environments {
    fn enter(&mut self, place: usize, alias: NodeId, budget: &mut Budget) -> Option<usize> {
        let mut parent = place;
        while parent != 0 {
            if !budget.take(1) {
                return None;
            }
            let (outer, id) = self.places[parent - 1];
            if id == alias {
                return None;
            }
            parent = outer;
        }
        Some(*self.place_ids.entry((place, alias)).or_insert_with(|| {
            self.places.push((place, alias));
            self.places.len()
        }))
    }
    fn bind(&mut self, b: Binding) -> usize {
        *self.ids.entry(b).or_insert_with(|| {
            self.frames.push(b);
            self.frames.len()
        })
    }
    fn lookup(&self, mut env: usize, symbol: SymbolId, budget: &mut Budget) -> Option<Term> {
        while env != 0 {
            if !budget.take(1) {
                return None;
            }
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
}
enum Children {
    One(Goal),
    Many(Vec<Goal>),
}
struct Budget {
    remaining: usize,
    exhausted: bool,
}
impl Budget {
    fn unparen<'a>(&mut self, mut ty: &'a TSType<'a>) -> Option<&'a TSType<'a>> {
        while let TSType::TSParenthesizedType(p) = ty {
            if !self.take(1) {
                return None;
            }
            ty = &p.type_annotation;
        }
        Some(ty)
    }
    fn take(&mut self, steps: usize) -> bool {
        if let Some(left) = self.remaining.checked_sub(steps) {
            self.remaining = left;
        } else {
            self.exhausted = true;
        }
        !self.exhausted
    }
}

#[derive(Default)]
struct Solver {
    envs: Environments,
    memo: HashMap<Goal, Class>,
    budget: Option<Budget>,
    incomplete: bool,
}
impl Solver {
    fn solve(&mut self, ctx: &TsCtx<'_>, goal: Goal) -> Class {
        let budget = self.budget.get_or_insert_with(|| Budget {
            // shortcut: 128 steps per syntax node per rule, revisit when ordinary aliases approach it.
            remaining: ctx.semantic.nodes().len().saturating_mul(128),
            exhausted: false,
        });
        if let Some(v) = evaluate(&mut self.memo, goal, budget, |g, b| {
            self.envs.plan(ctx, g, b)
        }) {
            v
        } else {
            self.incomplete = true;
            Class::None
        }
    }
}

struct Equation {
    plan: Plan,
    parents: Vec<Goal>,
    value: Class,
    queued: bool,
}

fn evaluate(
    memo: &mut HashMap<Goal, Class>,
    goal: Goal,
    budget: &mut Budget,
    mut plan: impl FnMut(Goal, &mut Budget) -> Plan,
) -> Option<Class> {
    if let Some(value) = memo.get(&goal) {
        return Some(*value);
    }
    let mut equations = HashMap::new();
    let mut discover = vec![goal];
    let mut queue = std::collections::VecDeque::new();
    while let Some(g) = discover.pop() {
        if !budget.take(1) {
            return None;
        }
        if equations.contains_key(&g) || memo.contains_key(&g) {
            continue;
        }
        let p = plan(g, budget);
        if !budget.take(p.children().len()) {
            return None;
        }
        discover.extend_from_slice(p.children());
        equations.insert(
            g,
            Equation {
                plan: p,
                parents: Vec::new(),
                value: Class::None,
                queued: true,
            },
        );
        queue.push_front(g);
    }
    let mut edges = Vec::new();
    for (&parent, eq) in &equations {
        edges.extend(eq.plan.children().iter().map(|&child| (child, parent)));
    }
    for (child, parent) in edges {
        if let Some(eq) = equations.get_mut(&child) {
            eq.parents.push(parent);
        }
    }
    // A recursive group starts at None; a changed answer schedules only its dependents.
    let mut values = Vec::new();
    while let Some(g) = queue.pop_front() {
        let eq = equations.get(&g).unwrap();
        if !budget.take(1 + eq.plan.children().len()) {
            return None;
        }
        let next = match &eq.plan {
            Plan::Done(c) => *c,
            Plan::Children(combine, _) => {
                values.clear();
                values.extend(
                    eq.plan
                        .children()
                        .iter()
                        .map(|c| memo.get(c).copied().unwrap_or_else(|| equations[c].value)),
                );
                combine.apply(&values)
            }
        };
        let eq = equations.get_mut(&g).unwrap();
        eq.queued = false;
        if next != eq.value {
            eq.value = next;
            if !budget.take(eq.parents.len()) {
                return None;
            }
            let parents = std::mem::take(&mut eq.parents);
            for parent in &parents {
                let dependent = equations.get_mut(parent).unwrap();
                if !dependent.queued {
                    dependent.queued = true;
                    queue.push_back(*parent);
                }
            }
            equations.get_mut(&g).unwrap().parents = parents;
        }
    }
    let result = equations[&goal].value;
    memo.extend(equations.into_iter().map(|(g, eq)| (g, eq.value)));
    Some(result)
}
impl Plan {
    fn children(&self) -> &[Goal] {
        match self {
            Self::Done(_) => &[],
            Self::Children(_, Children::One(g)) => std::slice::from_ref(g),
            Self::Children(_, Children::Many(gs)) => gs,
        }
    }
    fn child(combine: Combine, goal: Goal) -> Self {
        Self::Children(combine, Children::One(goal))
    }
    fn many(combine: Combine, goals: Vec<Goal>) -> Self {
        Self::Children(combine, Children::Many(goals))
    }
}
impl Environments {
    fn reference(
        &mut self,
        ctx: &TsCtx<'_>,
        g: Goal,
        r: &TSTypeReference<'_>,
        budget: &mut Budget,
    ) -> Plan {
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
                && let Some(term) = self.lookup(g.term.env, sym, budget)
            {
                return Plan::child(Combine::First, Goal { term, ..g });
            }
            let Some(id) = scopes
                .symbol_declarations(sym)
                .take_while(|_| budget.take(1))
                .find(|id| is_type_declaration(ctx.semantic.nodes().kind(*id)))
            else {
                return Plan::Done(Class::None);
            };
            match ctx.semantic.nodes().kind(id) {
                AstKind::TSInterfaceDeclaration(i)
                    if g.query == Query::Value
                        && i.extends.is_empty()
                        && empty(&i.body.body, budget)
                        && scopes
                            .symbol_declarations(sym)
                            .take_while(|_| budget.take(1))
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
                    return self.alias(ctx, g, a, args, budget);
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
    fn alias(
        &mut self,
        ctx: &TsCtx<'_>,
        g: Goal,
        a: &oxc::ast::ast::TSTypeAliasDeclaration<'_>,
        args: &[TSType<'_>],
        budget: &mut Budget,
    ) -> Plan {
        let mut env = 0;
        let mut place = 0;
        if let Some(params) = &a.type_parameters {
            let Some(inner) = self.enter(g.term.place, a.node_id.get(), budget) else {
                return Plan::Done(Class::None);
            };
            place = inner;
            if !budget.take(params.params.len()) {
                return Plan::Done(Class::None);
            }
            for (index, p) in params.params.iter().enumerate() {
                let (arg, arg_env, arg_place) = if let Some(arg) = args.get(index) {
                    (arg, g.term.env, g.term.place)
                } else if let Some(arg) = &p.default {
                    (arg, env, place)
                } else {
                    return Plan::Done(Class::None);
                };
                let value = self.argument(ctx, arg, arg_env, arg_place, budget);
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
        Plan::child(
            combine,
            Goal {
                term: Term {
                    node: a.type_annotation.node_id(),
                    env,
                    place,
                },
                query,
            },
        )
    }
    fn argument(
        &mut self,
        ctx: &TsCtx<'_>,
        ty: &TSType<'_>,
        env: usize,
        place: usize,
        budget: &mut Budget,
    ) -> Term {
        if let Some(TSType::TSTypeReference(r)) = budget.unparen(ty)
            && let TSTypeName::IdentifierReference(i) = &r.type_name
            && let Some(sym) = symbol(ctx, i)
            && let Some(value) = self.lookup(env, sym, budget)
        {
            return value;
        }
        Term {
            node: ty.node_id(),
            env,
            place,
        }
    }
    fn plan(&mut self, ctx: &TsCtx<'_>, g: Goal, budget: &mut Budget) -> Plan {
        let q = g.query;
        let kind = ctx.semantic.nodes().kind(g.term.node);
        let width = match kind {
            AstKind::TSUnionType(n) => n.types.len(),
            AstKind::TSIntersectionType(n) => n.types.len(),
            AstKind::TSTypeLiteral(n) => n.members.len(),
            _ => 0,
        };
        if !budget.take(width) {
            return Plan::Done(Class::None);
        }
        match (kind, q) {
            (AstKind::TSTypeReference(r), _) => self.reference(ctx, g, r, budget),
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
            (AstKind::TSTypeLiteral(n), Query::Value) if empty(&n.members, budget) => {
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

fn empty(members: &[TSSignature<'_>], budget: &mut Budget) -> bool {
    members.iter().all(|m| {
        if !budget.take(1) {
            return false;
        }
        let TSSignature::TSPropertySignature(p) = m else {
            return false;
        };
        p.optional
            && p.type_annotation.as_ref().is_some_and(|a| {
                let Some(mut ty) = budget.unparen(&a.type_annotation) else {
                    return false;
                };
                while let TSType::TSTypeOperatorType(op) = ty {
                    if !budget.take(1) || op.operator != TSTypeOperatorOperator::Readonly {
                        return false;
                    }
                    let Some(inner) = budget.unparen(&op.type_annotation) else {
                        return false;
                    };
                    ty = inner;
                }
                matches!(ty, TSType::TSNeverKeyword(_))
            })
    })
}

#[derive(Default)]
pub(super) struct Resolution(RefCell<Solver>);
impl Resolution {
    pub(super) fn incomplete(&self) -> bool {
        self.0.borrow().incomplete
    }
    pub(super) fn report(
        &self,
        findings: &mut crate::engine::Findings,
        offset: usize,
        matched: bool,
    ) {
        if std::mem::take(&mut self.0.borrow_mut().incomplete) {
            findings.report(offset, Some("type too complex to judge"));
        } else if matched {
            findings.report(offset, None);
        }
    }
    fn query(&self, ctx: &TsCtx<'_>, node: NodeId, query: Query) -> Class {
        self.0.borrow_mut().solve(
            ctx,
            Goal {
                term: Term {
                    node,
                    env: 0,
                    place: 0,
                },
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
    use super::{Budget, Class, Combine, Goal, Plan, Query, Term, evaluate};
    use oxc::syntax::node::NodeId;
    use std::collections::HashMap;

    #[test]
    fn exhausted_budget_reports_each_unjudged_alias() {
        use super::super::no_unknown_type_aliases::{RULE, check_aliases};
        use crate::engine::{Findings, Rule, Run, TsCtx, run};

        fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
            let resolution = super::Resolution::default();
            resolution.0.borrow_mut().budget = Some(Budget {
                remaining: 2,
                exhausted: false,
            });
            check_aliases(ctx, findings, &resolution);
        }
        let rules = [Rule {
            run: Run::Ts(check),
            ..RULE
        }];
        let hits = run(
            &rules,
            [(
                "types.ts",
                b"type Clean = string;\ntype X = Clean;\ntype Y = number;".as_slice(),
            )],
        );
        assert_eq!(
            hits.iter()
                .map(|f| (f.rule, f.line, f.col, f.note.as_deref()))
                .collect::<Vec<_>>(),
            [
                (RULE.id, 2, 6, Some("type too complex to judge")),
                (RULE.id, 3, 6, Some("type too complex to judge"))
            ]
        );
    }

    #[test]
    fn each_shared_goal_is_expanded_once_on_a_deep_work_stack() {
        let depth = 10_000;
        let goal = |i| Goal {
            term: Term {
                node: NodeId::new(i),
                env: 0,
                place: 0,
            },
            query: Query::UnknownAlias,
        };
        let mut calls = 0;
        let mut budget = Budget {
            remaining: usize::MAX,
            exhausted: false,
        };
        let mut memo = HashMap::new();
        let mut plan = |g: Goal, _: &mut Budget| {
            calls += 1;
            assert!(calls <= depth + 1, "a shared goal was expanded twice");
            let i = g.term.node.index();
            if i == 0 {
                Plan::Done(Class::Broad)
            } else if i.is_multiple_of(2) {
                Plan::many(Combine::Any, vec![goal(i - 1), goal(i - 1)])
            } else {
                Plan::child(Combine::First, goal(i - 1))
            }
        };
        assert!(evaluate(&mut memo, goal(depth), &mut budget, &mut plan) == Some(Class::Broad));
        for i in 0..depth {
            assert!(evaluate(&mut memo, goal(i), &mut budget, &mut plan) == Some(Class::Broad));
        }
        assert_eq!(calls, depth + 1);
    }
}
