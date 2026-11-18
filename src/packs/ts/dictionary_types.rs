use super::function_types::bindings::{symbol, type_declarations};
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
    AliasBody(NodeId),
    Dictionary,
    Any,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Query {
    Interface(SymbolId),
    PlainAlias,
    UnsafeDictionary,
    ObjectInput,
    Top,
    WideObject,
    WideKey,
    WideRecord,
    NarrowRecord,
    IsObject,
    UnknownParameter,
    Never,
    Empty,
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
    syntax_only: bool,
    frames: Vec<Binding>,
    ids: HashMap<Binding, usize>,
    forwarded: HashMap<NodeId, Option<NodeId>>,
    places: Vec<(usize, NodeId, Query)>,
    place_ids: HashMap<(usize, NodeId, Query), usize>,
}
impl Environments {
    fn enter(
        &mut self,
        place: usize,
        alias: NodeId,
        query: Query,
        budget: &mut Budget,
    ) -> Option<usize> {
        if let Some(&inner) = self.place_ids.get(&(place, alias, query)) {
            return Some(inner);
        }
        let mut parent = place;
        while parent != 0 {
            if !budget.take(1) {
                return None;
            }
            let (outer, id, active_query) = self.places[parent - 1];
            if id == alias && active_query == query {
                return None;
            }
            parent = outer;
        }
        Some(
            *self
                .place_ids
                .entry((place, alias, query))
                .or_insert_with(|| {
                    self.places.push((place, alias, query));
                    self.places.len()
                }),
        )
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
    Not,
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
            Self::Not if first == Class::None => Class::Broad,
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
struct Budget(Option<usize>);
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
        self.0 = self.0.and_then(|left| left.checked_sub(steps));
        self.0.is_some()
    }
}

#[derive(Default)]
struct Solver {
    envs: Environments,
    memo: HashMap<Goal, Class>,
    budget: Option<Budget>,
}
impl Solver {
    fn solve(&mut self, ctx: &TsCtx<'_>, goal: Goal) -> Judged<Class> {
        // shortcut: 128 steps per syntax node per rule, revisit when ordinary aliases approach it.
        let budget = self
            .budget
            .get_or_insert_with(|| Budget(Some(ctx.semantic.nodes().len().saturating_mul(128))));
        evaluate(&mut self.memo, goal, budget, |g, b| {
            self.envs.plan(ctx, g, b)
        })
        .ok_or(Unjudged)
    }
}

struct Equation {
    plan: Plan,
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
    let mut parents: HashMap<Goal, Vec<Goal>> = HashMap::new();
    let mut discover = vec![(goal, false)];
    let mut queue = std::collections::VecDeque::new();
    while let Some((g, expanded)) = discover.pop() {
        if expanded {
            queue.push_back(g);
            continue;
        }
        if !budget.take(1) {
            return None;
        }
        if equations.contains_key(&g) || memo.contains_key(&g) {
            continue;
        }
        let p = plan(g, budget);
        budget.0?;
        for &child in p.children() {
            if !memo.contains_key(&child) {
                parents.entry(child).or_default().push(g);
            }
        }
        discover.push((g, true));
        discover.extend(p.children().iter().rev().map(|&child| (child, false)));
        equations.insert(
            g,
            Equation {
                plan: p,
                value: Class::None,
                queued: true,
            },
        );
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
            let Some(dependents) = parents.get(&g) else {
                continue;
            };
            if !budget.take(dependents.len()) {
                return None;
            }
            for parent in dependents {
                let dependent = equations.get_mut(parent).unwrap();
                if !dependent.queued {
                    dependent.queued = true;
                    queue.push_back(*parent);
                }
            }
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
    fn interface_goal(ctx: &TsCtx<'_>, sym: SymbolId) -> Goal {
        Goal {
            term: Term {
                node: ctx.semantic.scoping().symbol_declaration(sym),
                env: 0,
                place: 0,
            },
            query: Query::Interface(sym),
        }
    }

    fn interface(ctx: &TsCtx<'_>, sym: SymbolId, budget: &mut Budget) -> Plan {
        let mut parents = Vec::new();
        for id in type_declarations(ctx, sym) {
            let AstKind::TSInterfaceDeclaration(i) = ctx.semantic.nodes().kind(id) else {
                return Plan::Done(Class::None);
            };
            if !budget.take(1 + i.extends.len())
                || !empty_members(
                    Self::interface_goal(ctx, sym),
                    &i.body.body,
                    budget,
                    &mut parents,
                )
            {
                return Plan::Done(Class::None);
            }
            for parent in &i.extends {
                let TSTypeName::IdentifierReference(name) = &parent.type_name else {
                    return Plan::Done(Class::None);
                };
                let Some(base) = symbol(ctx, name) else {
                    return Plan::Done(Class::None);
                };
                if ctx.semantic.scoping().symbol_flags(base).is_interface() {
                    parents.push(Self::interface_goal(ctx, base));
                } else if let Some(AstKind::TSTypeAliasDeclaration(alias)) =
                    type_declarations(ctx, base)
                        .next()
                        .map(|id| ctx.semantic.nodes().kind(id))
                    && alias.type_parameters.is_none()
                    && parent.type_arguments.is_none()
                {
                    parents.push(
                        Self::interface_goal(ctx, base).child(&alias.type_annotation, Query::Empty),
                    );
                } else {
                    return Plan::Done(Class::None);
                }
            }
        }
        if parents.is_empty() {
            Plan::Done(Class::Broad)
        } else {
            Plan::many(Combine::All, parents)
        }
    }

    fn forwarding_target(ctx: &TsCtx<'_>, id: NodeId, budget: &mut Budget) -> Option<NodeId> {
        let nodes = ctx.semantic.nodes();
        let alias = nodes.kind(id).as_ts_type_alias_declaration()?;
        let params = &alias.type_parameters.as_ref()?.params;
        let TSType::TSTypeReference(r) = budget.unparen(&alias.type_annotation)? else {
            return None;
        };
        let TSTypeName::IdentifierReference(name) = &r.type_name else {
            return None;
        };
        let args = &r.type_arguments.as_ref()?.params;
        if params.len() != args.len() || !budget.take(args.len()) {
            return None;
        }
        for (param, arg) in params.iter().zip(args) {
            let TSType::TSTypeReference(arg) = arg else {
                return None;
            };
            let TSTypeName::IdentifierReference(name) = &arg.type_name else {
                return None;
            };
            if param.default.is_some()
                || arg.type_arguments.is_some()
                || symbol(ctx, name) != param.name.symbol_id.get()
            {
                return None;
            }
        }
        let target = type_declarations(ctx, symbol(ctx, name)?).next()?;
        let dest = nodes.kind(target).as_ts_type_alias_declaration()?;
        let target_params = &dest.type_parameters.as_ref()?.params;
        (target_params.len() == args.len() && target_params.iter().all(|p| p.default.is_none()))
            .then_some(target)
    }

    fn forwarded(
        &mut self,
        ctx: &TsCtx<'_>,
        mut id: NodeId,
        budget: &mut Budget,
    ) -> Option<NodeId> {
        let mut path = Vec::new();
        let end = loop {
            if let Some(&cached) = self.forwarded.get(&id) {
                break cached;
            }
            if !budget.take(1) {
                return None;
            }
            let Some(next) = Self::forwarding_target(ctx, id, budget) else {
                break Some(id);
            };
            self.forwarded.insert(id, None);
            path.push(id);
            id = next;
        };
        for hop in path {
            self.forwarded.insert(hop, end);
        }
        end
    }

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
            if self.syntax_only {
                return Plan::Done(Class::None);
            }
            if scopes.symbol_flags(sym).is_type_parameter()
                && let Some(term) = self.lookup(g.term.env, sym, budget)
            {
                return Plan::child(Combine::First, Goal { term, ..g });
            }
            if matches!(g.query, Query::Value | Query::Empty)
                && scopes.symbol_flags(sym).is_interface()
            {
                return Plan::child(Combine::First, Self::interface_goal(ctx, sym));
            }
            let Some(id) = type_declarations(ctx, sym)
                .next()
                .and_then(|id| self.forwarded(ctx, id, budget))
            else {
                return Plan::Done(Class::None);
            };
            match ctx.semantic.nodes().kind(id) {
                AstKind::TSTypeAliasDeclaration(a) => {
                    return self.alias(ctx, g, a, args, budget);
                }
                _ => return Plan::Done(Class::None),
            }
        }
        if name.name == "Record"
            && let [key, value] = args
        {
            match g.query {
                Query::WideRecord => {
                    return Plan::many(
                        Combine::All,
                        vec![g.child(key, Query::WideKey), g.child(value, Query::Top)],
                    );
                }
                Query::NarrowRecord => {
                    return Plan::child(Combine::Not, g.child(value, Query::Top));
                }
                _ => {}
            }
        }
        let (index, query, combine) = match (name.name.as_str(), g.query) {
            (
                "Readonly" | "NonNullable" | "Partial" | "Required",
                Query::Target | Query::AliasTarget | Query::Value | Query::UnsafeDictionary,
            )
            | ("Promise" | "PromiseLike", Query::UnknownReturn)
            | ("Readonly", Query::WideRecord | Query::NarrowRecord)
            | ("Pick" | "Omit", Query::UnsafeDictionary) => (0, g.query, Combine::First),
            ("Record", Query::UnsafeDictionary) => (1, Query::Value, Combine::First),
            ("Record", Query::Target | Query::AliasTarget) if args.is_empty() => {
                return Plan::Done(Class::Dictionary);
            }
            ("Record", Query::Target | Query::AliasTarget) => (0, Query::Key, Combine::Dictionary),
            ("PropertyKey", Query::Key | Query::WideKey) => return Plan::Done(Class::Broad),
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
        let mut body = Term {
            node: a.type_annotation.node_id(),
            env: 0,
            place: 0,
        };
        if let Some(params) = &a.type_parameters {
            if matches!(
                g.query,
                Query::UnknownParameter
                    | Query::Never
                    | Query::Empty
                    | Query::Top
                    | Query::WideObject
                    | Query::WideKey
                    | Query::WideRecord
                    | Query::NarrowRecord
                    | Query::IsObject
            ) {
                return Plan::Done(Class::None);
            }
            let Some(inner) = self.enter(g.term.place, a.node_id.get(), g.query, budget) else {
                return Plan::Done(Class::None);
            };
            body.place = inner;
            if !budget.take(params.params.len()) {
                return Plan::Done(Class::None);
            }
            for (index, p) in params.params.iter().enumerate() {
                let (arg, base) = if let Some(arg) = args.get(index) {
                    (arg, g.term)
                } else if let Some(arg) = &p.default {
                    (arg, body)
                } else {
                    return Plan::Done(Class::None);
                };
                let value = self.argument(ctx, arg, base, budget);
                body.env = self.bind(Binding {
                    outer: body.env,
                    symbol: p.name.symbol_id.get().unwrap(),
                    value,
                });
            }
        }
        let query = annotation_query(g.query);
        let combine = if g.query == Query::Target && a.type_parameters.is_some() {
            Combine::OnlyDictionary
        } else {
            Combine::First
        };
        Plan::child(combine, Goal { term: body, query })
    }
    fn argument(
        &mut self,
        ctx: &TsCtx<'_>,
        ty: &TSType<'_>,
        base: Term,
        budget: &mut Budget,
    ) -> Term {
        if let Some(TSType::TSTypeReference(r)) = budget.unparen(ty)
            && let TSTypeName::IdentifierReference(i) = &r.type_name
            && let Some(sym) = symbol(ctx, i)
            && let Some(value) = self.lookup(base.env, sym, budget)
        {
            return value;
        }
        Term {
            node: ty.node_id(),
            ..base
        }
    }
    fn plain_alias(ctx: &TsCtx<'_>, g: Goal, kind: AstKind<'_>, budget: &mut Budget) -> Plan {
        let AstKind::TSTypeAliasDeclaration(alias) = kind else {
            return Plan::Done(Class::None);
        };
        if alias.type_parameters.is_some() {
            return Plan::Done(Class::None);
        }
        if let Some(TSType::TSTypeReference(r)) = budget.unparen(&alias.type_annotation)
            && let TSTypeName::IdentifierReference(i) = &r.type_name
            && let Some(sym) = symbol(ctx, i)
        {
            if r.type_arguments.is_some() {
                return Plan::Done(Class::None);
            }
            return type_declarations(ctx, sym)
                .next()
                .map_or(Plan::Done(Class::None), |node| {
                    Plan::child(
                        Combine::First,
                        Goal {
                            term: Term { node, ..g.term },
                            ..g
                        },
                    )
                });
        }
        Plan::Done(Class::AliasBody(g.term.node))
    }
    fn plan(&mut self, ctx: &TsCtx<'_>, g: Goal, budget: &mut Budget) -> Plan {
        let q = g.query;
        if let Query::Interface(sym) = q {
            return Self::interface(ctx, sym, budget);
        }
        let kind = ctx.semantic.nodes().kind(g.term.node);
        if q == Query::PlainAlias {
            return Self::plain_alias(ctx, g, kind, budget);
        }
        if let AstKind::TSTypeLiteral(n) = kind
            && matches!(
                q,
                Query::UnsafeDictionary | Query::Target | Query::AliasTarget
            )
            && !budget.take(n.members.len())
        {
            return Plan::Done(Class::None);
        }
        match (kind, q) {
            (AstKind::TSTypeReference(r), _) => self.reference(ctx, g, r, budget),
            (AstKind::TSParenthesizedType(p), _) => {
                Plan::child(Combine::First, g.child(&p.type_annotation, q))
            }
            (AstKind::TSTypeOperator(p), _)
                if p.operator == TSTypeOperatorOperator::Readonly
                    && !matches!(
                        q,
                        Query::Top
                            | Query::WideObject
                            | Query::WideRecord
                            | Query::WideKey
                            | Query::NarrowRecord
                            | Query::UnknownParameter
                    ) =>
            {
                Plan::child(Combine::First, g.child(&p.type_annotation, q))
            }
            _ => Self::classify(kind, g, budget),
        }
    }
    fn classify(kind: AstKind<'_>, g: Goal, budget: &mut Budget) -> Plan {
        let q = g.query;
        match (kind, q) {
            (
                AstKind::TSUnknownKeyword(_),
                Query::UnknownParameter | Query::UnknownReturn | Query::UnknownAlias,
            )
            | (AstKind::TSNeverKeyword(_), Query::Never)
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
                | Query::UnknownParameter
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
            (AstKind::TSTypeLiteral(n), Query::Value | Query::Empty) => {
                let mut children = Vec::new();
                if !empty_members(g, &n.members, budget, &mut children) {
                    Plan::Done(Class::None)
                } else if children.is_empty() {
                    Plan::Done(Class::Broad)
                } else {
                    Plan::many(Combine::All, children)
                }
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
            _ => Self::assertion_shape(kind, g, budget),
        }
    }
    fn assertion_shape(kind: AstKind<'_>, g: Goal, budget: &mut Budget) -> Plan {
        let q = g.query;
        match (kind, q) {
            (AstKind::TSAnyKeyword(_) | AstKind::TSUnknownKeyword(_), Query::Top)
            | (AstKind::TSObjectKeyword(_), Query::WideObject)
            | (
                AstKind::TSStringKeyword(_)
                | AstKind::TSNumberKeyword(_)
                | AstKind::TSSymbolKeyword(_),
                Query::WideKey,
            )
            | (
                AstKind::TSArrayType(_)
                | AstKind::TSConstructorType(_)
                | AstKind::TSFunctionType(_)
                | AstKind::TSMappedType(_)
                | AstKind::TSObjectKeyword(_)
                | AstKind::TSTupleType(_),
                Query::IsObject,
            ) => Plan::Done(Class::Broad),
            (AstKind::TSUnionType(n), Query::WideKey) => Plan::many(
                Combine::All,
                n.types.iter().map(|t| g.child(t, q)).collect(),
            ),
            (AstKind::TSIntersectionType(n), Query::IsObject) => Plan::many(
                Combine::All,
                n.types.iter().map(|t| g.child(t, q)).collect(),
            ),
            (AstKind::TSTypeLiteral(n), Query::IsObject) if !n.members.is_empty() => {
                Plan::Done(Class::Broad)
            }
            (AstKind::TSTypeLiteral(n), Query::WideRecord) => match n.members.as_slice() {
                [TSSignature::TSIndexSignature(i)] => Plan::many(
                    Combine::All,
                    vec![
                        g.child(&i.parameter.type_annotation.type_annotation, Query::WideKey),
                        g.child(&i.type_annotation.type_annotation, Query::Top),
                    ],
                ),
                _ => Plan::Done(Class::None),
            },
            (AstKind::TSTypeLiteral(n), Query::NarrowRecord)
                if budget.take(n.members.len())
                    && n.members
                        .iter()
                        .any(|m| !matches!(m, TSSignature::TSIndexSignature(_))) =>
            {
                Plan::Done(Class::Broad)
            }
            _ => Plan::Done(Class::None),
        }
    }
}

fn empty_members(
    g: Goal,
    members: &[TSSignature<'_>],
    budget: &mut Budget,
    children: &mut Vec<Goal>,
) -> bool {
    members.iter().all(|m| {
        if !budget.take(1) {
            return false;
        }
        let TSSignature::TSPropertySignature(p) = m else {
            return false;
        };
        let Some(annotation) = &p.type_annotation else {
            return false;
        };
        children.push(g.child(&annotation.type_annotation, Query::Never));
        p.optional
    })
}

// Naming an object or mapped annotation changes its written-form widening verdict.
fn annotation_query(query: Query) -> Query {
    if query == Query::Target {
        Query::AliasTarget
    } else {
        query
    }
}

#[derive(Default)]
pub(super) struct Resolution(RefCell<Solver>);
#[derive(Clone, Copy)]
pub(super) struct Unjudged;
pub(super) type Judged<T> = Result<T, Unjudged>;

pub(super) fn report(findings: &mut crate::engine::Findings, offset: usize, answer: Judged<bool>) {
    match answer {
        Ok(true) => findings.report(offset, None),
        Err(Unjudged) => findings.report(offset, Some("type too complex to judge")),
        Ok(false) => {}
    }
}
impl Resolution {
    fn query(&self, ctx: &TsCtx<'_>, node: NodeId, query: Query) -> Judged<Class> {
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
    pub(super) fn plain_alias<'a>(&self, ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> &'a TSType<'a> {
        if let TSType::TSTypeReference(r) = ty.without_parenthesized()
            && r.type_arguments.is_none()
            && let TSTypeName::IdentifierReference(i) = &r.type_name
            && let Some(sym) = symbol(ctx, i)
            && ctx.semantic.scoping().symbol_flags(sym).is_type_alias()
            && let Some(decl) = type_declarations(ctx, sym).next()
            && let Ok(Class::AliasBody(id)) = self.query(ctx, decl, Query::PlainAlias)
            && let AstKind::TSTypeAliasDeclaration(alias) = ctx.semantic.nodes().kind(id)
        {
            return &alias.type_annotation;
        }
        ty
    }
    pub(super) fn unsafe_dictionary(&self, ctx: &TsCtx<'_>, kind: AstKind<'_>) -> Judged<bool> {
        if !matches!(
            kind,
            AstKind::TSTypeReference(_) | AstKind::TSTypeLiteral(_) | AstKind::TSMappedType(_)
        ) {
            return Ok(false);
        }
        self.query(ctx, kind.node_id(), Query::UnsafeDictionary)
            .map(|v| v != Class::None)
    }
    pub(super) fn target(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Judged<Class> {
        self.query(ctx, ty.node_id(), Query::Target)
    }
    pub(super) fn informative(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Judged<bool> {
        self.query(ctx, ty.node_id(), Query::Value)
            .map(|v| v == Class::None)
    }
    pub(super) fn object_input(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Judged<bool> {
        self.query(ctx, ty.node_id(), Query::ObjectInput)
            .map(|v| v != Class::None)
    }
    pub(super) fn assertion_shape(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>, query: Query) -> bool {
        let value = self.query(ctx, ty.node_id(), query).unwrap_or_else(|_| {
            // Exhaustion preserves the assertion rule's verdict on the finite written type.
            let mut syntax = Environments {
                syntax_only: true,
                ..Environments::default()
            };
            let goal = Goal {
                term: Term {
                    node: ty.node_id(),
                    env: 0,
                    place: 0,
                },
                query,
            };
            evaluate(
                &mut HashMap::new(),
                goal,
                &mut Budget(Some(usize::MAX)),
                |g, b| syntax.plan(ctx, g, b),
            )
            .unwrap()
        });
        value != Class::None
    }
    pub(super) fn unknown_parameter(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Judged<bool> {
        self.query(ctx, ty.node_id(), Query::UnknownParameter)
            .map(|v| v != Class::None)
    }
    pub(super) fn unknown_alias(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Judged<bool> {
        self.query(ctx, ty.node_id(), Query::UnknownAlias)
            .map(|v| v != Class::None)
    }
    pub(super) fn unknown_return(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Judged<bool> {
        self.query(ctx, ty.node_id(), Query::UnknownReturn)
            .map(|v| v != Class::None)
    }
}

#[cfg(test)]
mod tests {
    use super::{Budget, Class, Combine, Goal, Plan, Query, Term, evaluate};
    use oxc::syntax::node::NodeId;
    use std::collections::HashMap;

    #[test]
    fn one_question_cannot_guard_or_cache_another() {
        use super::Solver;
        use crate::engine::{Findings, Rule, Run, TsCtx, run};
        use oxc::ast::AstKind;

        fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
            let nodes = ctx.semantic.nodes();
            let interface = nodes
                .iter()
                .find_map(|n| match n.kind() {
                    AstKind::TSInterfaceDeclaration(i) => i.id.symbol_id.get(),
                    _ => None,
                })
                .unwrap();
            let queries = [
                Query::Interface(interface),
                Query::PlainAlias,
                Query::UnsafeDictionary,
                Query::ObjectInput,
                Query::UnknownParameter,
                Query::UnknownAlias,
                Query::UnknownReturn,
                Query::Target,
                Query::AliasTarget,
                Query::Key,
                Query::Value,
                Query::Empty,
                Query::Never,
                Query::Top,
                Query::WideObject,
                Query::WideKey,
                Query::WideRecord,
                Query::NarrowRecord,
                Query::IsObject,
            ];
            let declaration = |name: &str| {
                nodes
                    .iter()
                    .find_map(|node| match node.kind() {
                        AstKind::TSTypeAliasDeclaration(a) if a.id.name == name => Some(a),
                        _ => None,
                    })
                    .unwrap()
            };
            let alias = declaration("Probe");
            let alias_id = declaration("A").node_id.get();
            for first in queries {
                for second in queries.into_iter().filter(|q| *q != first) {
                    let decl = alias.node_id.get();
                    let goal = |query| Goal {
                        query,
                        term: Term {
                            node: if query == Query::PlainAlias {
                                decl
                            } else {
                                alias.type_annotation.node_id()
                            },
                            env: 0,
                            place: 0,
                        },
                    };
                    let mut shared = Solver::default();
                    let places = &mut shared.envs;
                    let mut budget = Budget(Some(100));
                    let active = places.enter(0, alias_id, first, &mut budget).unwrap();
                    assert!(places.enter(active, alias_id, first, &mut budget).is_none());
                    assert!(
                        places
                            .enter(active, alias_id, second, &mut budget)
                            .is_some()
                    );
                    assert!(shared.solve(ctx, goal(first)).is_ok());
                    let alone = Solver::default().solve(ctx, goal(second)).ok().unwrap();
                    assert!(shared.solve(ctx, goal(second)).ok() == Some(alone));
                    let mut inside = goal(second);
                    inside.term.place = active;
                    assert!(shared.solve(ctx, inside).ok() == Some(alone));
                }
            }
            findings.report(0, None);
        }
        let rule = Rule {
            run: Run::Ts(check),
            ..super::super::no_unsafe_dictionary_type::RULE
        };
        for source in [
            "interface I {} type A = Record<string, unknown>; type Probe = A;",
            "interface I {} type A<T> = T; type Probe = A<Record<string, unknown>>;",
            "interface I {} type A<T=Record<string, unknown>> = T; type Probe = A;",
        ] {
            assert_eq!(
                run(
                    std::slice::from_ref(&rule),
                    [("types.ts", source.as_bytes())]
                )
                .len(),
                1
            );
        }
    }

    #[test]
    fn borderline_recursive_goals_have_reproducible_answers() {
        let goal = |i| Goal {
            term: Term {
                node: NodeId::new(i),
                env: 0,
                place: 0,
            },
            query: Query::UnknownAlias,
        };
        let run = |limit| {
            let mut memo = HashMap::new();
            let mut budget = Budget(Some(limit));
            evaluate(&mut memo, goal(0), &mut budget, |g, _| {
                let i = g.term.node.index();
                let links = [[1, 4], [0, 2], [0, 3], [3, 3], [5, 3]];
                links.get(i).map_or(Plan::Done(Class::Broad), |children| {
                    Plan::many(Combine::Any, children.iter().copied().map(goal).collect())
                })
            })
        };
        let boundary = (1..2000).find(|&limit| run(limit).is_some()).unwrap();
        for _ in 0..20 {
            assert!(run(boundary) == Some(Class::Broad));
            assert!(run(boundary - 1).is_none());
        }
    }

    #[test]
    fn repeated_guards_reuse_an_unjudged_origin() {
        use super::super::no_known_value_widening::{RULE, check_with_resolution};
        use crate::engine::{Findings, Rule, Run, TsCtx, run};
        use std::{
            fmt::Write as _,
            time::{Duration, Instant},
        };

        fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
            let resolution = super::Resolution::default();
            resolution.0.borrow_mut().budget = Some(Budget(Some(0)));
            check_with_resolution(ctx, findings, &resolution);
        }
        let mut source = String::from(
            "declare function guard(v: unknown): v is string;\ndeclare const root: string;\nconst v0 = root;\n",
        );
        for i in 1..=4000 {
            writeln!(source, "const v{i} = v{};", i - 1).unwrap();
        }
        source.push_str(&"guard(v4000);\n".repeat(4000));
        let start = Instant::now();
        let hits = run(
            &[Rule {
                run: Run::Ts(check),
                ..RULE
            }],
            [("guard.ts", source.as_bytes())],
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(hits.len(), 4000);
        for (i, hit) in hits.iter().enumerate() {
            assert_eq!(
                (hit.line, hit.col, hit.note.as_deref()),
                (4004 + i, 7, Some("type too complex to judge"))
            );
        }
    }

    #[test]
    fn exhausted_alias_lookup_keeps_the_syntactic_assertion_verdict() {
        use super::super::no_widen_then_assert::{RULE, check_with_resolution};
        use crate::engine::{Findings, Rule, Run, TsCtx, run};
        fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
            let resolution = super::Resolution::default();
            resolution.0.borrow_mut().budget = Some(Budget(Some(0)));
            check_with_resolution(ctx, findings, &resolution);
        }
        let source = "type Wide = Record<string, unknown>;\nconst value: Wide = { id: 1 };\nconst picked = value as { id: number };";
        assert!(
            run(
                &[Rule {
                    run: Run::Ts(check),
                    ..RULE
                }],
                [("alias.ts", source.as_bytes())]
            )
            .is_empty()
        );
    }

    #[test]
    fn exhausted_budget_reports_each_unjudged_alias() {
        use super::super::no_unknown_type_aliases::{RULE, check_aliases};
        use crate::engine::{Findings, Rule, Run, TsCtx, run};

        fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
            let resolution = super::Resolution::default();
            resolution.0.borrow_mut().budget = Some(Budget(Some(2)));
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
        let mut budget = Budget(Some(usize::MAX));
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
