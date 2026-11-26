use super::function_types::bindings::{declarations, symbol, type_declarations};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{
            IdentifierReference, Statement, TSNamespaceDeclaration, TSNamespaceDeclarationBody,
            TSSignature, TSType, TSTypeName, TSTypeOperatorOperator, TSTypeParameterDeclaration,
        },
    },
    syntax::{
        node::{GetNodeId, NodeId},
        scope::ScopeId,
        symbol::{SymbolFlags, SymbolId},
    },
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Class {
    None,
    Broad,
    AliasBody(NodeId),
    Declaration(SymbolId, usize),
    Syntax(Term),
    Dictionary,
    Any,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Query {
    Interface(SymbolId),
    PlainAlias,
    Identity,
    UnsafeDictionary,
    ContainedDictionary,
    NestedDictionary,
    BoundValue,
    ObjectInput,
    Top,
    WideObject,
    WideKey,
    WideRecord,
    NarrowRecord,
    IsObject,
    Never,
    UnknownAlias,
    UnknownReturn,
    Target,
    AliasTarget,
    Key,
    Value,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Term {
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
    fn wide_record(self, key: &TSType<'_>, value: &TSType<'_>) -> Plan {
        Plan::many(
            Combine::All,
            vec![
                self.child(key, Query::WideKey),
                self.child(value, Query::Top),
            ],
        )
    }
    fn root(node: NodeId, query: Query) -> Self {
        Self {
            term: Term {
                node,
                env: 0,
                place: 0,
            },
            query,
        }
    }
    fn dictionary_value(self, ty: &TSType<'_>) -> Self {
        self.child(
            ty,
            if self.query == Query::NestedDictionary {
                Query::BoundValue
            } else {
                Query::Value
            },
        )
    }
    fn nested<'a, T: GetNodeId + 'a>(self, types: impl Iterator<Item = &'a T>) -> Plan {
        Plan::many(
            Combine::First,
            types
                .map(|ty| self.at(ty.node_id(), Query::NestedDictionary))
                .collect(),
        )
    }

    fn child(self, ty: &TSType<'_>, query: Query) -> Self {
        self.at(ty.node_id(), query)
    }

    fn at(self, node: NodeId, query: Query) -> Self {
        Self {
            term: Term { node, ..self.term },
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
struct DictionaryMembers {
    indexes: Vec<NodeId>,
    properties: Vec<NodeId>,
}

#[derive(Default)]
struct Environments {
    frames: Vec<Binding>,
    resolved_names: HashMap<NodeId, Option<ResolvedName>>,
    enclosing_namespaces: HashMap<ScopeId, Option<ScopeId>>,
    namespace_exports: HashMap<Vec<SymbolId>, NamespaceExports>,
    namespace_implicit_exports: HashMap<NodeId, bool>,
    interface_indexes: HashMap<NodeId, Vec<NodeId>>,
    literal_members: HashMap<NodeId, DictionaryMembers>,
    applications: HashMap<(NodeId, usize, usize, Option<NodeId>), Term>,
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
    Not,
    OnlyBound,
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
            Self::OnlyBound if values.get(1) == Some(&Class::None) => values[0],
            Self::Not if first == Class::None => Class::Broad,
            Self::Any if first != Class::None => Class::Broad,
            Self::Dictionary if first != Class::None => Class::Dictionary,
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
    remaining: Option<usize>,
}
impl Budget {
    fn new(steps: usize) -> Self {
        Self {
            remaining: Some(steps),
        }
    }

    fn for_file<'a>(budget: &'a mut Option<Self>, ctx: &TsCtx<'_>) -> &'a mut Self {
        budget.get_or_insert_with(|| Self::new(ctx.semantic.nodes().len().saturating_mul(128)))
    }

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
        self.remaining = self.remaining.and_then(|left| left.checked_sub(steps));
        self.remaining.is_some()
    }
}

#[derive(Default)]
struct Solver {
    syntax_children: HashMap<NodeId, Vec<NodeId>>,
    envs: Environments,
    memo: HashMap<Goal, Class>,
    budget: Option<Budget>,
    #[cfg(test)]
    test_allowance: Option<usize>,
}
impl Solver {
    fn charge_identity(&mut self, ctx: &TsCtx<'_>, steps: usize) -> Judged<()> {
        let budget = self.budget.as_mut().ok_or(Unjudged)?;
        ctx.record_work(steps.min(budget.remaining.unwrap_or(0)));
        budget.take(steps).then_some(()).ok_or(Unjudged)
    }

    fn syntax_children(&mut self, ctx: &TsCtx<'_>, root: NodeId) -> Judged<Vec<NodeId>> {
        let nodes = ctx.semantic.nodes();
        if !self.syntax_children.contains_key(&root) {
            let mut children = HashMap::<NodeId, Vec<NodeId>>::from([(root, Vec::new())]);
            for index in root.index() + 1..nodes.len() {
                self.charge_identity(ctx, 1)?;
                let child = NodeId::new(index);
                let parent = nodes.parent_id(child);
                if parent < root {
                    break;
                }
                children.entry(parent).or_default().push(child);
                children.entry(child).or_default();
            }
            self.syntax_children.extend(children);
        }
        self.charge_identity(ctx, self.syntax_children[&root].len())?;
        Ok(self.syntax_children[&root].clone())
    }

    fn pair_signature_parameters(
        &mut self,
        ctx: &TsCtx<'_>,
        left: &mut Term,
        right: &mut Term,
    ) -> Judged<bool> {
        fn parameters(kind: AstKind<'_>) -> Option<&TSTypeParameterDeclaration<'_>> {
            match kind {
                AstKind::TSFunctionType(n) => n.type_parameters.as_deref(),
                AstKind::TSCallSignatureDeclaration(n) => n.type_parameters.as_deref(),
                AstKind::TSConstructorType(n) => n.type_parameters.as_deref(),
                AstKind::TSConstructSignatureDeclaration(n) => n.type_parameters.as_deref(),
                AstKind::TSMethodSignature(n) => n.type_parameters.as_deref(),
                _ => None,
            }
        }
        let nodes = ctx.semantic.nodes();
        let (Some(a), Some(b)) = (
            parameters(nodes.kind(left.node)),
            parameters(nodes.kind(right.node)),
        ) else {
            return Ok(true);
        };
        if a.params.len() != b.params.len() {
            return Ok(false);
        }
        self.charge_identity(ctx, a.params.len())?;
        for (x, y) in a.params.iter().zip(&b.params) {
            let identity = Goal::root(x.name.node_id.get(), Query::Identity).term;
            for (term, parameter) in [(&mut *left, x), (&mut *right, y)] {
                term.env = self.envs.bind(Binding {
                    outer: term.env,
                    symbol: parameter.name.symbol_id.get().unwrap(),
                    value: identity,
                });
            }
        }
        Ok(true)
    }

    fn solve(&mut self, ctx: &TsCtx<'_>, goal: Goal) -> Judged<Class> {
        // shortcut: 128 steps per syntax node and a minimum of 128 per candidate; revisit if ordinary inputs exhaust both.
        let budget = Budget::for_file(&mut self.budget, ctx);
        let before = budget.remaining.unwrap_or(0);
        let result = evaluate(&mut self.memo, goal, budget, |g, b| {
            self.envs.plan(ctx, g, b)
        });
        ctx.record_work(before - budget.remaining.unwrap_or(0));
        result.ok_or(Unjudged)
    }
}

struct Equation {
    plan: Plan,
    value: Class,
    queued: bool,
}

fn plan_root(
    memo: &HashMap<Goal, Class>,
    budget: &mut Budget,
    plan: impl FnOnce(&mut Budget) -> Plan,
) -> Option<Plan> {
    let available = budget.remaining.unwrap_or(0);
    let mut planning = Budget::new(available);
    let root = plan(&mut planning);
    let Some(remaining) = planning.remaining else {
        budget.remaining = None;
        return None;
    };
    budget.remaining = budget.remaining.map(|_| remaining);
    let children = root.children();
    if children.is_empty() {
        let value = match root {
            Plan::Done(value) => value,
            Plan::Children(combine, _) => combine.apply(&[]),
        };
        return Some(Plan::Done(value));
    }
    if !budget.take(children.len()) {
        return None;
    }
    if let Plan::Children(combine, Children::One(child)) = &root {
        if let Some(value) = memo.get(child) {
            return budget
                .take(2 + children.len())
                .then(|| Plan::Done(combine.apply(std::slice::from_ref(value))));
        }
        return Some(root);
    }
    if let Plan::Children(combine, _) = &root
        && let Some(values) = children
            .iter()
            .map(|child| memo.get(child).copied())
            .collect::<Option<Vec<_>>>()
    {
        if !budget.take(2 + children.len()) {
            return None;
        }
        let value = combine.apply(&values);
        return Some(Plan::Done(value));
    }
    Some(root)
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
    let root = plan_root(memo, budget, |planning| plan(goal, planning))?;
    if let Plan::Done(value) = root {
        memo.insert(goal, value);
        return Some(value);
    }
    let mut first = Some(root);
    let mut equations = HashMap::new();
    let mut parents: HashMap<Goal, Vec<Goal>> = HashMap::new();
    if !budget.take(1) {
        return None;
    }
    let mut discover = vec![(goal, false)];
    let mut queue = std::collections::VecDeque::new();
    while let Some((g, expanded)) = discover.pop() {
        if expanded {
            queue.push_back(g);
            continue;
        }
        if equations.contains_key(&g) || memo.contains_key(&g) {
            continue;
        }
        let p = first.take().unwrap_or_else(|| plan(g, budget));
        budget.remaining?;
        if !budget.take(p.children().len()) {
            return None;
        }
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
fn type_declaration(
    ctx: &TsCtx<'_>,
    symbol: SymbolId,
    budget: &mut Budget,
    wanted: impl Fn(AstKind<'_>) -> bool,
) -> Option<NodeId> {
    let first = ctx.semantic.scoping().symbol_declaration(symbol);
    if wanted(ctx.semantic.nodes().kind(first)) {
        return Some(first);
    }
    if !budget.take(ctx.semantic.scoping().symbol_redeclarations(symbol).len()) {
        return None;
    }
    type_declarations(ctx, symbol).find(|&id| wanted(ctx.semantic.nodes().kind(id)))
}
fn alias_declaration(ctx: &TsCtx<'_>, sym: SymbolId, budget: &mut Budget) -> Option<NodeId> {
    type_declaration(ctx, sym, budget, |kind| {
        matches!(kind, AstKind::TSTypeAliasDeclaration(_))
    })
}
fn name_root<'a>(
    mut name: &'a TSTypeName<'a>,
    members: &mut Vec<&'a str>,
    budget: &mut Budget,
) -> Option<&'a IdentifierReference<'a>> {
    while let TSTypeName::QualifiedName(prefix) = name {
        if !budget.take(1) {
            return None;
        }
        members.push(prefix.right.name.as_str());
        name = &prefix.left;
    }
    let TSTypeName::IdentifierReference(root) = name else {
        return None;
    };
    Some(root)
}
fn import_equals(ctx: &TsCtx<'_>, id: SymbolId, budget: &mut Budget) -> Option<NodeId> {
    let flags = ctx.semantic.scoping().symbol_flags(id);
    if !flags.is_import() && !flags.is_type_import() {
        return None;
    }
    type_declaration(ctx, id, budget, |kind| {
        matches!(kind, AstKind::TSImportEqualsDeclaration(_))
    })
}
#[derive(Clone)]
enum ResolvedName {
    Single(SymbolId),
    Multiple(Rc<[SymbolId]>),
}

impl ResolvedName {
    fn from_symbols(symbols: Vec<SymbolId>) -> Self {
        if let [id] = symbols[..] {
            Self::Single(id)
        } else {
            Self::Multiple(symbols.into())
        }
    }

    fn symbols(&self) -> &[SymbolId] {
        match self {
            Self::Single(id) => std::slice::from_ref(id),
            Self::Multiple(ids) => ids,
        }
    }
}

type NamespaceExports = HashMap<String, Vec<SymbolId>>;

fn implicit_namespace_exports(
    ctx: &TsCtx<'_>,
    ns: &TSNamespaceDeclaration<'_>,
    budget: &mut Budget,
) -> Option<bool> {
    let mut ambient = ns.declare || ctx.program.source_type.is_typescript_definition();
    if !ambient {
        for ancestor in ctx.ancestors(ns.node_id()) {
            if !budget.take(1) {
                return None;
            }
            ambient = match ancestor.kind() {
                AstKind::TSNamespaceDeclaration(ns) => ns.declare,
                AstKind::TSExternalModuleDeclaration(module) => module.declare,
                AstKind::TSGlobalDeclaration(global) => global.declare,
                _ => false,
            };
            if ambient {
                break;
            }
        }
    }
    if ambient && let TSNamespaceDeclarationBody::TSModuleBlock(block) = &ns.body {
        if !budget.take(block.body.len()) {
            return None;
        }
        // An export list disables implicit exports for this declaration's body.
        return Some(!block.body.iter().any(|statement| {
            matches!(
                statement,
                Statement::ExportNamedDeclaration(_)
                    | Statement::ExportFromDeclaration(_)
                    | Statement::ExportAllDeclaration(_)
                    | Statement::TSExportAssignment(_)
            )
        }));
    }
    Some(ambient)
}

impl Environments {
    fn namespace_exported(
        &mut self,
        ctx: &TsCtx<'_>,
        ns: &TSNamespaceDeclaration<'_>,
        decl: NodeId,
        budget: &mut Budget,
    ) -> Option<bool> {
        let nodes = ctx.semantic.nodes();
        if matches!(
            nodes.parent_kind(decl),
            AstKind::ExportDeclaration(_) | AstKind::TSNamespaceDeclaration(_)
        ) {
            return Some(true);
        }
        if matches!(nodes.kind(decl), AstKind::TSImportEqualsDeclaration(_)) {
            return Some(false);
        }
        let implicit = match self.namespace_implicit_exports.entry(ns.node_id()) {
            std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                *entry.insert(implicit_namespace_exports(ctx, ns, budget)?)
            }
        };
        Some(implicit)
    }

    fn namespace_exports(
        &mut self,
        ctx: &TsCtx<'_>,
        symbols: &[SymbolId],
        budget: &mut Budget,
    ) -> Option<NamespaceExports> {
        let nodes = ctx.semantic.nodes();
        let scopes = ctx.semantic.scoping();
        let mut exports = NamespaceExports::new();
        for (id, _) in symbols.iter().flat_map(|&sym| declarations(ctx, sym)) {
            if !budget.take(1) {
                return None;
            }
            let AstKind::TSNamespaceDeclaration(ns) = nodes.kind(id) else {
                continue;
            };
            let bindings = scopes.get_bindings(ns.scope_id.get()?);
            if !budget.take(bindings.len()) {
                return None;
            }
            for (name, &candidate) in bindings {
                for (decl, _) in declarations(ctx, candidate) {
                    if !budget.take(1) {
                        return None;
                    }
                    if self.namespace_exported(ctx, ns, decl, budget)? {
                        exports.entry(name.to_string()).or_default().push(candidate);
                        break;
                    }
                }
            }
        }
        Some(exports)
    }

    fn interface(&mut self, ctx: &TsCtx<'_>, g: Goal, sym: SymbolId, budget: &mut Budget) -> Plan {
        let nodes = ctx.semantic.nodes();
        let args = match nodes.kind(g.term.node) {
            AstKind::TSInterfaceHeritage(h) => h.type_arguments.as_deref(),
            AstKind::TSTypeReference(r) => r.type_arguments.as_deref(),
            _ => None,
        }
        .map_or(&[][..], |a| a.params.as_slice());
        let mut parents = Vec::new();
        if !budget.take(ctx.semantic.scoping().symbol_redeclarations(sym).len()) {
            return Plan::Done(Class::None);
        }
        for id in type_declarations(ctx, sym) {
            let AstKind::TSInterfaceDeclaration(i) = nodes.kind(id) else {
                return Plan::Done(Class::None);
            };
            let body = if let Some(params) = &i.type_parameters {
                let Some(term) = self.application(ctx, g, id, params, args, budget) else {
                    return Plan::Done(Class::None);
                };
                Goal { term, ..g }
            } else {
                Goal::root(id, g.query)
            };
            if !budget.take(1 + i.extends.len())
                || !empty_members(body, &i.body.body, budget, &mut parents)
            {
                return Plan::Done(Class::None);
            }
            for parent in &i.extends {
                parents.push(body.at(parent.node_id.get(), Query::Value));
            }
        }
        if parents.is_empty() {
            Plan::Done(Class::Broad)
        } else {
            Plan::many(Combine::All, parents)
        }
    }

    fn interface_dictionary(
        &mut self,
        ctx: &TsCtx<'_>,
        g: Goal,
        symbols: &[SymbolId],
        args: &[TSType<'_>],
        budget: &mut Budget,
    ) -> Plan {
        let mut children = Vec::new();
        if !symbols
            .iter()
            .all(|&sym| budget.take(ctx.semantic.scoping().symbol_redeclarations(sym).len()))
        {
            return Plan::Done(Class::None);
        }
        for id in symbols.iter().flat_map(|&sym| type_declarations(ctx, sym)) {
            let Some(interface) = ctx.semantic.nodes().kind(id).as_ts_interface_declaration()
            else {
                continue;
            };
            let (body, indexes) = if let Some(params) = &interface.type_parameters {
                let Some(term) = self.application(ctx, g, id, params, args, budget) else {
                    return Plan::Done(Class::None);
                };
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    self.interface_indexes.entry(id)
                {
                    if !budget.take(interface.body.body.len()) {
                        return Plan::Done(Class::None);
                    }
                    entry.insert(
                        interface
                            .body
                            .body
                            .iter()
                            .filter_map(|m| match m {
                                TSSignature::TSIndexSignature(i) => {
                                    Some(i.type_annotation.type_annotation.node_id())
                                }
                                _ => None,
                            })
                            .collect(),
                    );
                }
                (Goal { term, ..g }, self.interface_indexes[&id].as_slice())
            } else {
                // Index signatures without bindings are checked where written.
                (Goal::root(id, g.query), &[][..])
            };
            if !budget.take(indexes.len() + interface.extends.len()) {
                return Plan::Done(Class::None);
            }
            children.extend(indexes.iter().map(|&node| body.at(node, Query::BoundValue)));
            children.extend(
                interface
                    .extends
                    .iter()
                    .map(|h| body.at(h.node_id.get(), g.query)),
            );
        }
        Plan::many(Combine::First, children)
    }

    fn forwarding_target(
        &mut self,
        ctx: &TsCtx<'_>,
        id: NodeId,
        budget: &mut Budget,
    ) -> Option<NodeId> {
        let nodes = ctx.semantic.nodes();
        let alias = nodes.kind(id).as_ts_type_alias_declaration()?;
        let TSType::TSTypeReference(r) = budget.unparen(&alias.type_annotation)? else {
            return None;
        };
        let TSTypeName::IdentifierReference(name) = &r.type_name else {
            return None;
        };
        let ResolvedName::Single(sym) = self.lexical_name(ctx, name, false, budget)? else {
            return None;
        };
        let target = alias_declaration(ctx, sym, budget)?;
        let dest = nodes.kind(target).as_ts_type_alias_declaration()?;
        if alias.type_parameters.is_none()
            && r.type_arguments.is_none()
            && dest.type_parameters.is_none()
        {
            return Some(target);
        }
        let params = &alias.type_parameters.as_ref()?.params;
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
        let mut path = std::collections::HashSet::new();
        let mut first = None;
        let end = loop {
            if let Some(&cached) = self.forwarded.get(&id) {
                break cached;
            }
            if !budget.take(1) {
                return None;
            }
            let Some(next) = self.forwarding_target(ctx, id, budget) else {
                budget.remaining?;
                break Some(id);
            };
            // A single hop into a cached tail needs no cycle set.
            if first.is_none() {
                first = Some(id);
            } else if first == Some(id) || !path.insert(id) {
                break None;
            }
            id = next;
        };
        for hop in first.into_iter().chain(path) {
            self.forwarded.insert(hop, end);
        }
        end
    }

    fn namespace_member(
        &mut self,
        ctx: &TsCtx<'_>,
        symbols: &[SymbolId],
        member: &str,
        budget: &mut Budget,
    ) -> Option<&[SymbolId]> {
        if !self.namespace_exports.contains_key(symbols) {
            let exports = self.namespace_exports(ctx, symbols, budget)?;
            self.namespace_exports.insert(symbols.to_vec(), exports);
        }
        let candidates = self.namespace_exports[symbols].get(member)?;
        budget.take(candidates.len() - 1).then_some(candidates)
    }

    fn namespace_symbols<'a>(
        &mut self,
        ctx: &TsCtx<'a>,
        mut ns: &'a TSNamespaceDeclaration<'a>,
        budget: &mut Budget,
    ) -> Option<Vec<SymbolId>> {
        let scopes = ctx.semantic.scoping();
        let nodes = ctx.semantic.nodes();
        let mut path = Vec::new();
        let root = loop {
            let sym = ns.id.symbol_id.get()?;
            let owner = scopes.get_node_id(scopes.symbol_scope_id(sym));
            let AstKind::TSNamespaceDeclaration(parent) = nodes.kind(owner) else {
                break sym;
            };
            if !self.namespace_exported(ctx, parent, ns.node_id(), budget)? {
                break sym;
            }
            if !budget.take(1) {
                return None;
            }
            path.push((ns.id.name.as_str(), sym));
            ns = parent;
        };
        let mut symbols = vec![root];
        for (member, local) in path.into_iter().rev() {
            if let [sym] = symbols[..]
                && scopes.symbol_redeclarations(sym).is_empty()
            {
                symbols[0] = local;
            } else {
                symbols = self
                    .namespace_member(ctx, &symbols, member, budget)?
                    .to_vec();
            }
        }
        Some(symbols)
    }

    fn enclosing_namespace(&mut self, ctx: &TsCtx<'_>, scope: ScopeId) -> Option<ScopeId> {
        let scopes = ctx.semantic.scoping();
        let root = scopes.root_scope_id();
        if scope == root
            || (scopes.scope_parent_id(scope) == Some(root)
                && !scopes.scope_flags(scope).is_ts_module_block())
        {
            return None;
        }
        if let Some(&cached) = self.enclosing_namespaces.get(&scope) {
            return cached;
        }
        let mut path = Vec::new();
        let mut enclosing = None;
        for scope in ctx.counted(scopes.scope_ancestors(scope)) {
            if let Some(&cached) = self.enclosing_namespaces.get(&scope) {
                enclosing = cached;
                break;
            }
            path.push(scope);
            if scopes.scope_flags(scope).is_ts_module_block()
                && matches!(
                    ctx.semantic.nodes().kind(scopes.get_node_id(scope)),
                    AstKind::TSNamespaceDeclaration(_)
                )
            {
                enclosing = Some(scope);
                break;
            }
        }
        self.enclosing_namespaces
            .extend(path.into_iter().map(|scope| (scope, enclosing)));
        enclosing
    }

    fn lexical_name(
        &mut self,
        ctx: &TsCtx<'_>,
        name: &IdentifierReference<'_>,
        qualifier: bool,
        budget: &mut Budget,
    ) -> Option<ResolvedName> {
        let scopes = ctx.semantic.scoping();
        let nodes = ctx.semantic.nodes();
        let bound = symbol(ctx, name);
        let mut enclosing =
            self.enclosing_namespace(ctx, nodes.get_node(name.node_id()).scope_id());
        if enclosing.is_none() {
            return bound.map(ResolvedName::Single);
        }
        let binding_scope = bound.map(|id| scopes.symbol_scope_id(id));
        let binding_namespace =
            binding_scope.and_then(|scope| self.enclosing_namespace(ctx, scope));
        while let Some(scope) = enclosing {
            if binding_namespace == enclosing && binding_scope != enclosing {
                break;
            }
            let ns = nodes
                .kind(scopes.get_node_id(scope))
                .as_ts_namespace_declaration()?;
            if !budget.take(1) {
                return None;
            }
            let symbols = self.namespace_symbols(ctx, ns, budget)?;
            let merged = symbols.len() > 1 || !scopes.symbol_redeclarations(symbols[0]).is_empty();
            let local = bound.filter(|_| binding_scope == enclosing);
            if merged
                && let Some(exports) =
                    self.namespace_member(ctx, &symbols, name.name.as_str(), budget)
                && local.is_none_or(|id| exports.contains(&id))
            {
                let exports: Vec<_> = exports
                    .iter()
                    .copied()
                    .filter(|&id| {
                        let flags = scopes.symbol_flags(id);
                        if qualifier {
                            flags.can_be_referenced_as_namespace()
                        } else {
                            flags.can_be_referenced_by_type()
                        }
                    })
                    .collect();
                if !exports.is_empty() {
                    return Some(ResolvedName::from_symbols(exports));
                }
            }
            if local.is_some() {
                break;
            }
            enclosing = scopes
                .scope_parent_id(scope)
                .and_then(|parent| self.enclosing_namespace(ctx, parent));
        }
        bound.map(ResolvedName::Single)
    }

    fn local_name(
        &mut self,
        ctx: &TsCtx<'_>,
        name: &TSTypeName<'_>,
        budget: &mut Budget,
    ) -> Option<ResolvedName> {
        use oxc::ast::ast::TSModuleReference;
        let mut members = Vec::new();
        let mut imports = std::collections::HashSet::new();
        let root = name_root(name, &mut members, budget)?;
        let mut resolved = self.lexical_name(ctx, root, !members.is_empty(), budget)?;
        let nodes = ctx.semantic.nodes();
        let scopes = ctx.semantic.scoping();
        loop {
            let symbols = resolved.symbols();
            if !budget.take(symbols.len() - 1) {
                return None;
            }
            let sym = *symbols.iter().rev().max_by_key(|&&id| {
                let flags = scopes.symbol_flags(id);
                (
                    flags.intersects(
                        SymbolFlags::Type | SymbolFlags::Import | SymbolFlags::TypeImport,
                    ),
                    flags.is_class(),
                )
            })?;
            if let Some(import) = import_equals(ctx, sym, budget)
                && let AstKind::TSImportEqualsDeclaration(i) = nodes.kind(import)
            {
                if !budget.take(1) || !imports.insert(sym) {
                    return None;
                }
                let root = match &i.module_reference {
                    TSModuleReference::IdentifierReference(i) => i,
                    TSModuleReference::QualifiedName(q) => {
                        members.push(q.right.name.as_str());
                        name_root(&q.left, &mut members, budget)?
                    }
                    TSModuleReference::ExternalModuleReference(_) => return None,
                };
                resolved = self.lexical_name(ctx, root, true, budget)?;
                continue;
            }
            let Some(member) = members.pop() else {
                let flags = scopes.symbol_flags(sym);
                if symbols.len() > 1 && (flags.is_interface() || flags.is_class()) {
                    let mut types = vec![sym];
                    types.extend(
                        symbols
                            .iter()
                            .copied()
                            .filter(|&id| id != sym && scopes.symbol_flags(id).is_interface()),
                    );
                    return Some(ResolvedName::from_symbols(types));
                }
                return Some(ResolvedName::Single(sym));
            };
            let candidates = self.namespace_member(ctx, symbols, member, budget)?;
            resolved = ResolvedName::from_symbols(candidates.to_vec());
        }
    }
    fn resolve_name(
        &mut self,
        ctx: &TsCtx<'_>,
        name: &TSTypeName<'_>,
        budget: &mut Budget,
    ) -> Option<ResolvedName> {
        if let Some(cached) = self.resolved_names.get(&name.node_id()) {
            return cached.clone();
        }
        let resolved = self.local_name(ctx, name, budget);
        budget.remaining?;
        self.resolved_names.insert(name.node_id(), resolved.clone());
        resolved
    }

    fn terminal_reference(g: Goal, symbol: SymbolId, has_arguments: bool) -> Plan {
        Plan::Done(match g.query {
            Query::Identity if !has_arguments => Class::Declaration(symbol, 0),
            Query::Identity => Class::Syntax(g.term),
            _ => Class::None,
        })
    }

    fn declaration_identity(
        &mut self,
        ctx: &TsCtx<'_>,
        g: Goal,
        symbol: SymbolId,
        args: &[TSType<'_>],
        budget: &mut Budget,
    ) -> Option<Class> {
        let id = type_declaration(ctx, symbol, budget, |kind| {
            matches!(kind, AstKind::Class(_) | AstKind::TSInterfaceDeclaration(_))
        })?;
        let params = match ctx.semantic.nodes().kind(id) {
            AstKind::TSInterfaceDeclaration(i) => i.type_parameters.as_deref(),
            AstKind::Class(c) => c.type_parameters.as_deref(),
            _ => None,
        };
        let env = if let Some(params) = params {
            self.application(ctx, g, id, params, args, budget)?.env
        } else {
            0
        };
        Some(Class::Declaration(symbol, env))
    }

    fn reference(
        &mut self,
        ctx: &TsCtx<'_>,
        g: Goal,
        type_name: &TSTypeName<'_>,
        type_arguments: Option<&oxc::ast::ast::TSTypeParameterInstantiation<'_>>,
        budget: &mut Budget,
    ) -> Plan {
        let args = type_arguments.map_or(&[][..], |a| a.params.as_slice());
        let scopes = ctx.semantic.scoping();
        if let Some(name) = self.resolve_name(ctx, type_name, budget) {
            let symbols = name.symbols();
            if !budget.take(symbols.len() - 1) {
                return Plan::Done(Class::None);
            }
            let sym = symbols[0];
            let flags = scopes.symbol_flags(sym);
            if flags.is_type_parameter() {
                let Some(term) = self.lookup(g.term.env, sym, budget) else {
                    return Self::terminal_reference(g, sym, !args.is_empty());
                };
                if g.query == Query::NestedDictionary {
                    return Plan::Done(Class::None);
                }
                return Plan::child(Combine::First, Goal { term, ..g });
            }
            if g.query == Query::Identity && (flags.is_interface() || flags.is_class()) {
                return Plan::Done(
                    self.declaration_identity(ctx, g, sym, args, budget)
                        .unwrap_or(Class::None),
                );
            }
            if g.query == Query::Value && flags.is_class() {
                // A merged interface does not replace the class's instance type.
                return Self::terminal_reference(g, sym, !args.is_empty());
            }
            let interface = symbols
                .iter()
                .find(|&&id| scopes.symbol_flags(id).is_interface());
            if let Some(&interface) = interface
                && let Some(id) = type_declaration(ctx, interface, budget, |kind| {
                    matches!(kind, AstKind::TSInterfaceDeclaration(_))
                })
            {
                let generic = ctx
                    .semantic
                    .nodes()
                    .kind(id)
                    .as_ts_interface_declaration()
                    .is_some_and(|i| i.type_parameters.is_some());
                if matches!(
                    g.query,
                    Query::UnsafeDictionary | Query::ContainedDictionary | Query::NestedDictionary
                ) && (generic
                    || matches!(
                        ctx.semantic.nodes().kind(g.term.node),
                        AstKind::TSInterfaceHeritage(_)
                    ))
                {
                    return self.interface_dictionary(ctx, g, symbols, args, budget);
                }
                if g.query == Query::Value {
                    let goal = |symbol, declaration| {
                        let query = Query::Interface(symbol);
                        if generic {
                            Goal { query, ..g }
                        } else {
                            Goal::root(declaration, query)
                        }
                    };
                    return match name {
                        ResolvedName::Single(_) => Plan::child(Combine::First, goal(sym, id)),
                        ResolvedName::Multiple(ids) => Plan::many(
                            Combine::All,
                            ids.iter()
                                .map(|&symbol| goal(symbol, scopes.symbol_declaration(symbol)))
                                .collect(),
                        ),
                    };
                }
            }
            if !flags.is_type_alias() {
                return Self::terminal_reference(g, sym, !args.is_empty());
            }
            let Some(id) =
                alias_declaration(ctx, sym, budget).and_then(|id| self.forwarded(ctx, id, budget))
            else {
                return Plan::Done(Class::None);
            };
            return match ctx.semantic.nodes().kind(id) {
                AstKind::TSTypeAliasDeclaration(a) => self.alias(ctx, g, a, args, budget),
                _ => Plan::Done(Class::None),
            };
        }
        if g.query == Query::Identity {
            return Plan::Done(Class::Syntax(g.term));
        }
        Self::builtin_reference(g, type_name, args)
    }

    fn builtin_reference(g: Goal, type_name: &TSTypeName<'_>, args: &[TSType<'_>]) -> Plan {
        let TSTypeName::IdentifierReference(name) = type_name else {
            return Plan::Done(Class::None);
        };
        let (index, query, combine) = match (name.name.as_str(), g.query) {
            (
                "Readonly" | "NonNullable" | "Partial" | "Required",
                Query::Target
                | Query::AliasTarget
                | Query::Value
                | Query::UnsafeDictionary
                | Query::ContainedDictionary
                | Query::NestedDictionary,
            )
            | ("Promise" | "PromiseLike", Query::UnknownReturn)
            | ("Readonly", Query::WideRecord | Query::NarrowRecord)
            | (
                "Pick" | "Omit",
                Query::UnsafeDictionary | Query::ContainedDictionary | Query::NestedDictionary,
            ) => (0, g.query, Combine::First),
            ("Record", Query::WideRecord) => {
                return match args {
                    [key, value] => g.wide_record(key, value),
                    _ => Plan::Done(Class::None),
                };
            }
            ("Record", Query::NarrowRecord) if args.len() == 2 => (1, Query::Top, Combine::Not),
            ("Record", Query::ContainedDictionary | Query::NestedDictionary) => {
                let Some(value) = args.get(1) else {
                    return Plan::Done(Class::None);
                };
                return Plan::many(
                    Combine::First,
                    vec![
                        g.dictionary_value(value),
                        g.child(value, Query::NestedDictionary),
                    ],
                );
            }
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
            let Some(bound) = self.application(ctx, g, a.node_id.get(), params, args, budget)
            else {
                return Plan::Done(Class::None);
            };
            body.env = bound.env;
            body.place = bound.place;
        }
        let query = annotation_query(g.query);
        Plan::child(Combine::First, Goal { term: body, query })
    }
    fn application(
        &mut self,
        ctx: &TsCtx<'_>,
        g: Goal,
        declaration: NodeId,
        params: &oxc::ast::ast::TSTypeParameterDeclaration<'_>,
        args: &[TSType<'_>],
        budget: &mut Budget,
    ) -> Option<Term> {
        let mut body = Term {
            node: declaration,
            env: 0,
            place: self.enter(g.term.place, declaration, g.query, budget)?,
        };
        let key = (
            declaration,
            body.place,
            g.term.env,
            args.first().map(TSType::node_id),
        );
        if let Some(&cached) = self.applications.get(&key) {
            return Some(cached);
        }
        if !budget.take(params.params.len()) {
            return None;
        }
        let mut parameter_values = HashMap::new();
        for (index, p) in params.params.iter().enumerate() {
            let (arg, base) = if let Some(arg) = args.get(index) {
                (arg, g.term)
            } else if let Some(arg) = &p.default {
                (
                    arg,
                    Term {
                        place: g.term.place,
                        ..body
                    },
                )
            } else {
                return None;
            };
            let value = if index >= args.len()
                && let TSType::TSTypeReference(r) = arg
                && let TSTypeName::IdentifierReference(name) = &r.type_name
                && let Some(value) = symbol(ctx, name).and_then(|s| parameter_values.get(&s))
            {
                *value
            } else {
                self.argument(ctx, arg, base, budget)
            };
            budget.remaining?;
            parameter_values.insert(p.name.symbol_id.get().unwrap(), value);
            body.env = self.bind(Binding {
                outer: body.env,
                symbol: p.name.symbol_id.get().unwrap(),
                value,
            });
        }
        budget.remaining?;
        self.applications.insert(key, body);
        Some(body)
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
    fn plain_alias(
        &mut self,
        ctx: &TsCtx<'_>,
        g: Goal,
        kind: AstKind<'_>,
        budget: &mut Budget,
    ) -> Plan {
        let AstKind::TSTypeAliasDeclaration(alias) = kind else {
            return Plan::Done(Class::None);
        };
        if alias.type_parameters.is_some() {
            return Plan::Done(Class::None);
        }
        if let Some(TSType::TSTypeReference(r)) = budget.unparen(&alias.type_annotation)
            && let TSTypeName::IdentifierReference(i) = &r.type_name
            && let Some(ResolvedName::Single(sym)) = self.lexical_name(ctx, i, false, budget)
        {
            if r.type_arguments.is_some() {
                return Plan::Done(Class::None);
            }
            return alias_declaration(ctx, sym, budget).map_or(Plan::Done(Class::None), |node| {
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
    fn literal(
        &mut self,
        g: Goal,
        n: &oxc::ast::ast::TSTypeLiteral<'_>,
        budget: &mut Budget,
    ) -> Plan {
        let q = g.query;
        if let std::collections::hash_map::Entry::Vacant(entry) =
            self.literal_members.entry(g.term.node)
        {
            if !budget.take(n.members.len()) {
                return Plan::Done(Class::None);
            }
            let mut members = DictionaryMembers::default();
            for member in &n.members {
                match member {
                    TSSignature::TSIndexSignature(i) => members
                        .indexes
                        .push(i.type_annotation.type_annotation.node_id()),
                    TSSignature::TSPropertySignature(p) => {
                        if let Some(a) = &p.type_annotation
                            && !a.type_annotation.is_keyword_or_literal()
                        {
                            members.properties.push(a.type_annotation.node_id());
                        }
                    }
                    _ => {}
                }
            }
            entry.insert(members);
        }
        let members = &self.literal_members[&g.term.node];
        if matches!(q, Query::Target | Query::AliasTarget) {
            return Plan::Done(if !members.indexes.is_empty() {
                Class::Dictionary
            } else if q == Query::Target && !n.members.is_empty() {
                Class::Broad
            } else {
                Class::None
            });
        }
        if q == Query::NarrowRecord {
            return Plan::Done(if n.members.len() > members.indexes.len() {
                Class::Broad
            } else {
                Class::None
            });
        }
        let properties = if q == Query::UnsafeDictionary {
            &[][..]
        } else {
            members.properties.as_slice()
        };
        if !budget.take(members.indexes.len() + properties.len()) {
            return Plan::Done(Class::None);
        }
        let value_query = if q == Query::NestedDictionary {
            Query::BoundValue
        } else {
            Query::Value
        };
        let children = members
            .indexes
            .iter()
            .map(|&n| (n, value_query))
            .chain(properties.iter().map(|&n| (n, Query::NestedDictionary)))
            .map(|(node, query)| Goal {
                term: Term { node, ..g.term },
                query,
            })
            .collect();
        Plan::many(Combine::First, children)
    }
    fn plan(&mut self, ctx: &TsCtx<'_>, g: Goal, budget: &mut Budget) -> Plan {
        let q = g.query;
        if let Query::Interface(sym) = q {
            return self.interface(ctx, g, sym, budget);
        }
        let nodes = ctx.semantic.nodes();
        let kind = nodes.kind(g.term.node);
        if q == Query::BoundValue {
            return Plan::many(
                Combine::OnlyBound,
                vec![
                    Goal {
                        query: Query::Value,
                        ..g
                    },
                    Goal::root(g.term.node, Query::Value),
                ],
            );
        }
        if q == Query::PlainAlias {
            return self.plain_alias(ctx, g, kind, budget);
        }
        if let AstKind::TSTypeLiteral(n) = kind
            && matches!(
                q,
                Query::UnsafeDictionary
                    | Query::ContainedDictionary
                    | Query::NestedDictionary
                    | Query::Target
                    | Query::AliasTarget
                    | Query::NarrowRecord
            )
        {
            return self.literal(g, n, budget);
        }
        let visits = match (kind, q) {
            (
                AstKind::TSUnionType(n),
                Query::Value
                | Query::Key
                | Query::UnknownReturn
                | Query::UnknownAlias
                | Query::ObjectInput
                | Query::ContainedDictionary
                | Query::NestedDictionary
                | Query::WideKey,
            ) => n.types.len(),
            (
                AstKind::TSIntersectionType(n),
                Query::Value
                | Query::IsObject
                | Query::ContainedDictionary
                | Query::NestedDictionary,
            ) => n.types.len(),
            (AstKind::TSTupleType(n), Query::ContainedDictionary | Query::NestedDictionary) => {
                n.element_types.len()
            }
            _ => 0,
        };
        if visits != 0 && !budget.take(visits) {
            return Plan::Done(Class::None);
        }
        match (kind, q) {
            (AstKind::TSTypeReference(r), _) => {
                self.reference(ctx, g, &r.type_name, r.type_arguments.as_deref(), budget)
            }
            (AstKind::TSInterfaceHeritage(h), _) => {
                self.reference(ctx, g, &h.type_name, h.type_arguments.as_deref(), budget)
            }
            (AstKind::TSParenthesizedType(p), _) => {
                Plan::child(Combine::First, g.child(&p.type_annotation, q))
            }
            (AstKind::TSTypeOperator(p), _) if p.operator == TSTypeOperatorOperator::Readonly => {
                Plan::child(Combine::First, g.child(&p.type_annotation, q))
            }
            (AstKind::BindingIdentifier(i), Query::Identity)
                if matches!(nodes.parent_kind(g.term.node), AstKind::TSTypeParameter(_))
                    && matches!(
                        nodes.parent_kind(nodes.parent_id(g.term.node)),
                        AstKind::TSTypeParameterDeclaration(_)
                    ) =>
            {
                let sym = i.symbol_id.get().unwrap();
                self.lookup(g.term.env, sym, budget).map_or_else(
                    || Plan::Done(Class::Declaration(sym, 0)),
                    |term| Plan::child(Combine::First, Goal { term, ..g }),
                )
            }
            (_, Query::Identity) => Plan::Done(Class::Syntax(g.term)),
            _ => Self::classify(kind, g, budget),
        }
    }
    fn classify(kind: AstKind<'_>, g: Goal, budget: &mut Budget) -> Plan {
        let q = g.query;
        match (kind, q) {
            (AstKind::TSUnknownKeyword(_), Query::UnknownReturn | Query::UnknownAlias)
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
                Query::Key | Query::WideKey,
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
            (AstKind::TSIntersectionType(n), Query::Value | Query::IsObject) => Plan::many(
                Combine::All,
                n.types.iter().map(|ty| g.child(ty, q)).collect(),
            ),
            (AstKind::TSTypeLiteral(n), Query::Value) => {
                let mut children = Vec::new();
                if !empty_members(g, &n.members, budget, &mut children) {
                    Plan::Done(Class::None)
                } else if children.is_empty() {
                    Plan::Done(Class::Broad)
                } else {
                    Plan::many(Combine::All, children)
                }
            }
            (AstKind::TSTypeLiteral(n), Query::IsObject) if !n.members.is_empty() => {
                Plan::Done(Class::Broad)
            }
            (AstKind::TSMappedType(_), Query::Target) => Plan::Done(Class::Dictionary),
            (AstKind::TSMappedType(n), Query::AliasTarget) => {
                Plan::child(Combine::Dictionary, g.child(&n.constraint, Query::Key))
            }
            (
                AstKind::TSMappedType(n),
                Query::UnsafeDictionary | Query::ContainedDictionary | Query::NestedDictionary,
            ) => n
                .type_annotation
                .as_ref()
                .map_or(Plan::Done(Class::None), |ty| {
                    Plan::child(Combine::First, g.child(ty, Query::Value))
                }),
            _ => Self::assertion_shape(kind, g),
        }
    }
    fn assertion_shape(kind: AstKind<'_>, g: Goal) -> Plan {
        let q = g.query;
        match (kind, q) {
            (AstKind::TSAnyKeyword(_) | AstKind::TSUnknownKeyword(_), Query::Top)
            | (AstKind::TSObjectKeyword(_), Query::WideObject)
            | (
                AstKind::TSArrayType(_)
                | AstKind::TSConstructorType(_)
                | AstKind::TSFunctionType(_)
                | AstKind::TSMappedType(_)
                | AstKind::TSObjectKeyword(_)
                | AstKind::TSTupleType(_),
                Query::IsObject,
            ) => Plan::Done(Class::Broad),
            (AstKind::TSUnionType(n), Query::ContainedDictionary | Query::NestedDictionary) => {
                g.nested(n.types.iter())
            }
            (
                AstKind::TSIntersectionType(n),
                Query::ContainedDictionary | Query::NestedDictionary,
            ) => g.nested(n.types.iter()),
            (AstKind::TSArrayType(n), Query::ContainedDictionary | Query::NestedDictionary) => {
                g.nested(std::iter::once(&n.element_type))
            }
            (AstKind::TSTupleType(n), Query::ContainedDictionary | Query::NestedDictionary) => {
                g.nested(n.element_types.iter())
            }
            (AstKind::TSNamedTupleMember(n), Query::NestedDictionary) => {
                Plan::child(Combine::First, g.at(n.element_type.node_id(), q))
            }
            (AstKind::TSOptionalType(n), Query::NestedDictionary) => {
                Plan::child(Combine::First, g.child(&n.type_annotation, q))
            }
            (AstKind::TSRestType(n), Query::NestedDictionary) => {
                Plan::child(Combine::First, g.child(&n.type_annotation, q))
            }
            (AstKind::TSUnionType(n), Query::WideKey) => Plan::many(
                Combine::All,
                n.types.iter().map(|t| g.child(t, q)).collect(),
            ),
            (AstKind::TSTypeLiteral(n), Query::WideRecord) => match n.members.as_slice() {
                [TSSignature::TSIndexSignature(i)] => g.wide_record(
                    &i.parameter.type_annotation.type_annotation,
                    &i.type_annotation.type_annotation,
                ),
                _ => Plan::Done(Class::None),
            },
            _ => Plan::Done(Class::None),
        }
    }
}

fn same_gap(text: &str, left: u32, left_end: u32, right: u32, right_end: u32) -> bool {
    let gap = |start: u32, end: u32| {
        text[start as usize..end as usize]
            .chars()
            .filter(|c| !c.is_whitespace())
    };
    gap(left, left_end).eq(gap(right, right_end))
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
        if !p.optional {
            return false;
        }
        children.push(g.child(&annotation.type_annotation, Query::Never));
        true
    })
}

// Naming an object or mapped annotation changes its written-form widening verdict.
fn annotation_query(query: Query) -> Query {
    match query {
        Query::Target => Query::AliasTarget,
        Query::UnsafeDictionary => Query::ContainedDictionary,
        other => other,
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
    pub(super) fn begin_candidate(&self) {
        let mut solver = self.0.borrow_mut();
        let allowance = 128;
        #[cfg(test)]
        let allowance = solver.test_allowance.unwrap_or(allowance);
        if let Some(budget) = &mut solver.budget {
            budget.remaining = Some(budget.remaining.unwrap_or(0).max(allowance));
        }
    }

    #[cfg(test)]
    fn with_budget(steps: usize, allowance: usize) -> Self {
        Self(RefCell::new(Solver {
            budget: Some(Budget::new(steps)),
            test_allowance: Some(allowance),
            ..Solver::default()
        }))
    }
    fn query(&self, ctx: &TsCtx<'_>, node: NodeId, query: Query) -> Judged<Class> {
        self.0.borrow_mut().solve(ctx, Goal::root(node, query))
    }
    pub(super) fn plain_alias<'a>(
        &self,
        ctx: &TsCtx<'a>,
        ty: &'a TSType<'a>,
    ) -> Judged<&'a TSType<'a>> {
        if let TSType::TSTypeReference(r) = ty.without_parenthesized()
            && r.type_arguments.is_none()
            && let TSTypeName::IdentifierReference(i) = &r.type_name
            && let Some(sym) = symbol(ctx, i)
            && ctx.semantic.scoping().symbol_flags(sym).is_type_alias()
            && let Some(decl) = {
                let mut solver = self.0.borrow_mut();
                let budget = Budget::for_file(&mut solver.budget, ctx);
                let before = budget.remaining.unwrap_or(0);
                let result = alias_declaration(ctx, sym, budget);
                ctx.record_work(before - budget.remaining.unwrap_or(0));
                budget.remaining.ok_or(Unjudged)?;
                result
            }
            && let Class::AliasBody(id) = self.query(ctx, decl, Query::PlainAlias)?
            && let AstKind::TSTypeAliasDeclaration(alias) = ctx.semantic.nodes().kind(id)
        {
            return Ok(&alias.type_annotation);
        }
        Ok(ty)
    }
    pub(super) fn is_bare_alias_use(&self, ctx: &TsCtx<'_>, kind: AstKind<'_>) -> bool {
        let AstKind::TSTypeReference(r) = kind else {
            return false;
        };
        r.type_arguments.is_none()
            && self
                .0
                .borrow()
                .envs
                .resolved_names
                .get(&r.type_name.node_id())
                .and_then(Option::as_ref)
                .is_some_and(|name| {
                    ctx.semantic
                        .scoping()
                        .symbol_flags(name.symbols()[0])
                        .is_type_alias()
                })
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
    pub(super) fn same_type<'a>(
        &self,
        ctx: &TsCtx<'a>,
        left: &'a TSType<'a>,
        right: &'a TSType<'a>,
    ) -> Judged<bool> {
        use oxc::span::GetSpan;
        let root = |ty: &TSType<'_>| Goal::root(ty.node_id(), Query::Identity);
        let mut pairs = vec![(root(left), root(right))];
        let mut seen = std::collections::HashSet::new();
        let mut solver = self.0.borrow_mut();
        let nodes = ctx.semantic.nodes();
        while let Some((left, right)) = pairs.pop() {
            if !seen.insert((left, right)) {
                continue;
            }
            let a = solver.solve(ctx, left)?;
            let b = solver.solve(ctx, right)?;
            let (mut a, mut b) = match (a, b) {
                (Class::Declaration(a, mut x), Class::Declaration(b, mut y)) if a == b => {
                    while x != y {
                        solver.charge_identity(ctx, 1)?;
                        let (Some(left_binding), Some(right_binding)) = (
                            x.checked_sub(1).map(|n| solver.envs.frames[n]),
                            y.checked_sub(1).map(|n| solver.envs.frames[n]),
                        ) else {
                            return Ok(false);
                        };
                        pairs.push((
                            Goal {
                                term: left_binding.value,
                                ..left
                            },
                            Goal {
                                term: right_binding.value,
                                ..right
                            },
                        ));
                        x = left_binding.outer;
                        y = right_binding.outer;
                    }
                    continue;
                }
                (Class::Declaration(..), _) | (_, Class::Declaration(..)) => return Ok(false),
                (a, b) => {
                    let syntax = |class, term| match class {
                        Class::Syntax(resolved) => resolved,
                        _ => term,
                    };
                    (syntax(a, left.term), syntax(b, right.term))
                }
            };
            let (ak, bk) = (nodes.kind(a.node), nodes.kind(b.node));
            if a == b {
                continue;
            }
            solver.charge_identity(ctx, 1)?;
            if std::mem::discriminant(&ak) != std::mem::discriminant(&bk) {
                return Ok(false);
            }
            if let (AstKind::IdentifierReference(x), AstKind::IdentifierReference(y)) = (ak, bk)
                && symbol(ctx, x) != symbol(ctx, y)
            {
                return Ok(false);
            }
            if !solver.pair_signature_parameters(ctx, &mut a, &mut b)? {
                return Ok(false);
            }
            let xs = solver.syntax_children(ctx, a.node)?;
            let ys = solver.syntax_children(ctx, b.node)?;
            if xs.len() != ys.len() {
                return Ok(false);
            }
            let (mut ax, mut bx) = (ak.span().start, bk.span().start);
            for (x, y) in xs.into_iter().zip(ys) {
                let (sx, sy) = (nodes.kind(x).span(), nodes.kind(y).span());
                if !same_gap(ctx.text, ax, sx.start.max(ax), bx, sy.start.max(bx)) {
                    return Ok(false);
                }
                ax = ax.max(sx.end);
                bx = bx.max(sy.end);
                pairs.push((
                    Goal {
                        term: Term { node: x, ..a },
                        ..left
                    },
                    Goal {
                        term: Term { node: y, ..b },
                        ..right
                    },
                ));
            }
            if !same_gap(
                ctx.text,
                ax,
                ak.span().end.max(ax),
                bx,
                bk.span().end.max(bx),
            ) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn assertion_shape(
        &self,
        ctx: &TsCtx<'_>,
        ty: &TSType<'_>,
        query: Query,
    ) -> Judged<bool> {
        self.query(ctx, ty.node_id(), query)
            .map(|value| value != Class::None)
    }
    pub(super) fn contains_unknown(&self, ctx: &TsCtx<'_>, ty: &TSType<'_>) -> Judged<bool> {
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
    fn a_plain_alias_of_unknown_stays_silent_at_zero_budget() {
        use crate::engine::run;
        let rule = assertion::<0, 0>();
        let source =
            "type Top = unknown;\nconst wide: unknown = 'ok';\nconst picked = wide as Top;";
        let hits = run(&[rule], [("plain.ts", source.as_bytes())]);
        assert!(
            hits.is_empty(),
            "{:?}",
            hits.iter()
                .map(|hit| (hit.rule, hit.line, hit.col, &hit.note))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn parameters_retain_identity_in_their_declaring_scope() {
        use crate::engine::{Findings, Rule, Run, TsCtx, run};
        use oxc::ast::AstKind;

        fn compare(ctx: &TsCtx<'_>, findings: &mut Findings) {
            let annotations: Vec<_> = ctx
                .semantic
                .nodes()
                .iter()
                .filter_map(|node| {
                    let AstKind::TSPropertySignature(property) = node.kind() else {
                        return None;
                    };
                    Some(&property.type_annotation.as_ref()?.type_annotation)
                })
                .collect();
            let [left, right] = annotations.as_slice() else {
                panic!("expected two test properties");
            };
            super::report(
                findings,
                0,
                super::Resolution::default().same_type(ctx, left, right),
            );
        }

        let rule = Rule {
            run: Run::Ts(compare),
            ..super::super::no_widen_then_assert::RULE
        };
        for (context, equal) in [
            (
                "function f<T>() { type Pair = { a: LEFT; b: RIGHT }; }",
                true,
            ),
            (
                "class C<T> { m() { type Pair = { a: LEFT; b: RIGHT }; } }",
                true,
            ),
            (
                "class C { m<T>() { type Pair = { a: LEFT; b: RIGHT }; } }",
                true,
            ),
            ("interface I<T> { a: LEFT; b: RIGHT }", true),
            ("type Pair<T> = { a: LEFT; b: RIGHT };", true),
            ("type Pair<T extends string> = { a: LEFT; b: RIGHT };", true),
            ("type Pair<T = number> = { a: LEFT; b: RIGHT };", true),
            (
                "function outer<T>() { type A = { a: LEFT }; function inner<T>() { type B = { b: RIGHT }; } }",
                false,
            ),
            (
                "function outer<T>() { type A = { a: LEFT }; function inner<U>() { type B = { b: RIGHT }; } }",
                false,
            ),
        ] {
            for shape in ["X", "X[]", "readonly X[]"] {
                for left in ["T", "Id<T>"] {
                    for right in ["T", "Id<T>"] {
                        let right = if context.contains("inner<U>") {
                            right.replace('T', "U")
                        } else {
                            right.to_owned()
                        };
                        let body = context
                            .replace("LEFT", &shape.replace('X', left))
                            .replace("RIGHT", &shape.replace('X', &right));
                        let source = format!("type Id<V> = V; {body}");
                        let hits = run(
                            std::slice::from_ref(&rule),
                            [("identity.ts", source.as_bytes())],
                        );
                        assert_eq!(
                            hits.iter()
                                .map(|h| (h.rule, h.note.as_deref()))
                                .collect::<Vec<_>>(),
                            if equal { vec![(rule.id, None)] } else { vec![] },
                            "{source}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn planning_refuses_member_walks_larger_than_the_remaining_budget() {
        use super::super::no_unsafe_dictionary_type::RULE;
        use crate::engine::{Findings, Rule, Run, TsCtx, run};
        fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
            for (id, node) in ctx.semantic.nodes().iter_enumerated() {
                if !matches!(
                    node.kind(),
                    oxc::ast::AstKind::TSUnionType(_) | oxc::ast::AstKind::TSTypeLiteral(_)
                ) {
                    continue;
                }
                let mut budget = Budget::new(1);
                let plan = super::Environments::default().plan(
                    ctx,
                    Goal::root(id, Query::ContainedDictionary),
                    &mut budget,
                );
                if budget.remaining.is_none() && plan.children().is_empty() {
                    findings.report(0, None);
                }
            }
        }
        let hits = run(
            &[Rule {
                run: Run::Ts(check),
                ..RULE
            }],
            [(
                "walk.ts",
                b"type A = { a: number; b: string }; type B = number | string;".as_slice(),
            )],
        );
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn an_interrupted_alias_walk_cannot_answer_a_shared_tail() {
        use super::super::no_unknown_type_aliases::RULE;
        use crate::engine::{Findings, Rule, Run, TsCtx, run};
        fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
            let ids: Vec<_> = ctx
                .semantic
                .nodes()
                .iter()
                .filter_map(|n| {
                    n.kind()
                        .as_ts_type_alias_declaration()
                        .map(|a| a.node_id.get())
                })
                .collect();
            let mut env = super::Environments::default();
            assert_eq!(env.forwarded(ctx, ids[0], &mut Budget::new(1)), None);
            for start in [ids[1], ids[0]] {
                assert_eq!(
                    env.forwarded(ctx, start, &mut Budget::new(10)),
                    Some(ids[2])
                );
            }
            findings.report(0, None);
        }
        let hits = run(
            &[Rule {
                run: Run::Ts(check),
                ..RULE
            }],
            [(
                "chain.ts",
                b"type A=B; type B=C; type C=unknown;".as_slice(),
            )],
        );
        assert_eq!(hits.len(), 1);
    }

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
                Query::UnknownAlias,
                Query::UnknownReturn,
                Query::Target,
                Query::AliasTarget,
                Query::Key,
                Query::Value,
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
                    let goal = |query| {
                        Goal::root(
                            if query == Query::PlainAlias {
                                decl
                            } else {
                                alias.type_annotation.node_id()
                            },
                            query,
                        )
                    };
                    let mut shared = Solver::default();
                    let places = &mut shared.envs;
                    let mut budget = Budget::new(100);
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
            let mut budget = Budget::new(limit);
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

    use crate::engine::{Rule, Run};

    fn guard<const STEPS: usize, const ALLOWANCE: usize>() -> Rule {
        use super::super::no_known_value_widening;
        Rule {
            run: Run::Ts(|ctx, findings| {
                no_known_value_widening::check_with_resolution(
                    ctx,
                    findings,
                    &super::Resolution::with_budget(STEPS, ALLOWANCE),
                );
            }),
            ..no_known_value_widening::RULE
        }
    }

    fn dictionary<const STEPS: usize, const ALLOWANCE: usize>() -> Rule {
        use super::super::no_unsafe_dictionary_type;
        Rule {
            run: Run::Ts(|ctx, findings| {
                no_unsafe_dictionary_type::check_with_resolution(
                    ctx,
                    findings,
                    &super::Resolution::with_budget(STEPS, ALLOWANCE),
                );
            }),
            ..no_unsafe_dictionary_type::RULE
        }
    }

    fn assertion<const STEPS: usize, const ALLOWANCE: usize>() -> Rule {
        use super::super::no_widen_then_assert;
        Rule {
            run: Run::Ts(|ctx, findings| {
                no_widen_then_assert::check_with_resolution(
                    ctx,
                    findings,
                    &super::Resolution::with_budget(STEPS, ALLOWANCE),
                );
            }),
            ..no_widen_then_assert::RULE
        }
    }

    fn alias<const STEPS: usize, const ALLOWANCE: usize>() -> Rule {
        use super::super::no_unknown_type_aliases;
        Rule {
            run: Run::Ts(|ctx, findings| {
                no_unknown_type_aliases::check_aliases(
                    ctx,
                    findings,
                    &super::Resolution::with_budget(STEPS, ALLOWANCE),
                );
            }),
            ..no_unknown_type_aliases::RULE
        }
    }

    #[test]
    fn a_later_guard_can_finish_a_shared_origin() {
        use crate::engine::run;
        use std::fmt::Write;
        let mut source = String::from("type P0 = unknown; type E0 = string;\n");
        for i in 1..=80 {
            writeln!(
                source,
                "type P{i} = P{prev}; type E{i} = E{prev};",
                prev = i - 1
            )
            .unwrap();
        }
        source.push_str("declare function slow(v: P80): v is string; declare function cheap(v: unknown): v is string;\ndeclare const root: E80; const shared = root;\nslow(shared);\ncheap(shared);");
        let hits = run(&[guard::<0, 128>()], [("shared.ts", source.as_bytes())]);
        assert_eq!(
            hits.iter()
                .map(|h| (h.line, h.col, h.note.as_deref()))
                .collect::<Vec<_>>(),
            [(84, 6, Some("type too complex to judge")), (85, 7, None)]
        );
    }

    #[test]
    fn repeated_guards_reuse_an_unjudged_origin() {
        use crate::engine::{run, run_with_work};
        use std::fmt::Write as _;

        for (origin, note) in [
            (
                "type Origin = string; declare const root: Origin;",
                Some("type too complex to judge"),
            ),
            ("declare const root: string;", None),
        ] {
            let mut source = format!(
                "declare function guard(v: unknown): v is string;\n{origin}\nconst v0 = root;\n",
            );
            for i in 1..=4000 {
                writeln!(source, "const v{i} = v{};", i - 1).unwrap();
            }
            source.push_str(&"guard(v4000);\n".repeat(4000));
            let (hits, work) = run_with_work(&[guard::<0, 0>()], "guard.ts", source.as_bytes());
            let cap = (source.len() / 3) as u64;
            assert!(work <= cap, "{work} work exceeds {cap}");
            assert_eq!(hits.len(), 4000);
            for (i, hit) in hits.iter().enumerate() {
                assert_eq!(
                    (hit.line, hit.col, hit.note.as_deref()),
                    (4004 + i, 7, note)
                );
            }
            if note.is_none() {
                let other = run(
                    &[guard::<{ usize::MAX }, 0>()],
                    [("guard.ts", source.as_bytes())],
                );
                assert!(
                    hits.iter()
                        .map(|h| (h.rule, &h.path, h.line, h.col, &h.note))
                        .eq(other
                            .iter()
                            .map(|h| (h.rule, &h.path, h.line, h.col, &h.note)))
                );
            }
        }
    }

    fn default_parameters() -> String {
        use std::fmt::Write as _;
        let mut params = String::new();
        for i in 0..128 {
            write!(params, ", T{i} = string").unwrap();
        }
        params
    }

    #[test]
    fn exhausted_dictionary_budget_keeps_the_later_record_red() {
        use crate::engine::run;
        let defaults = default_parameters();
        let text = "type Wrap<TDEFAULTS> = T[];\ntype Stress = Wrap<string>;\nconst value: Record<string, unknown> = {id: 1};".replace("DEFAULTS", &defaults);
        for (runner, note) in [
            (dictionary::<0, 128>(), Some("type too complex to judge")),
            (dictionary::<{ usize::MAX }, 128>(), None),
        ] {
            let hits = run(&[runner], [("later.ts", text.as_bytes())]);
            let actual: Vec<_> = hits
                .iter()
                .map(|h| (h.line, h.col, h.note.as_deref()))
                .collect();
            let expected = if note.is_some() {
                vec![(2, 15, note), (3, 14, None)]
            } else {
                vec![(3, 14, None)]
            };
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn unresolved_assertion_reports_at_the_ordinary_finding_position() {
        use crate::engine::run;
        let source = "type Wide = Record<string, unknown>;\nconst value: Wide = { id: 1 };\nconst picked = value as { id: number };";
        for (runner, note) in [
            (assertion::<{ usize::MAX }, 0>(), None),
            (assertion::<0, 0>(), Some("type too complex to judge")),
        ] {
            let hits = run(&[runner], [("alias.ts", source.as_bytes())]);
            assert_eq!(
                hits.iter()
                    .map(|h| (h.rule, h.line, h.col, h.note.as_deref()))
                    .collect::<Vec<_>>(),
                [(super::super::no_widen_then_assert::RULE.id, 3, 16, note)]
            );
        }
    }

    #[test]
    fn an_exhausted_assertion_allows_the_later_plain_assertion() {
        use crate::engine::run;
        let defaults = default_parameters();
        let source = "type Key<TDEFAULTS> = T;\nconst stress: Record<Key<string>, unknown> = {id: 1};\nconst first = stress as {id: number};\nconst plain: Record<string, unknown> = {id: 1};\nconst next = plain as {id: number};".replace("DEFAULTS", &defaults);
        for (runner, note) in [
            (assertion::<1, 128>(), Some("type too complex to judge")),
            (assertion::<{ usize::MAX }, 128>(), None),
        ] {
            let hits = run(&[runner], [("pair.ts", source.as_bytes())]);
            assert_eq!(
                hits.iter()
                    .map(|h| (h.rule, h.line, h.col, h.note.as_deref()))
                    .collect::<Vec<_>>(),
                [
                    (super::super::no_widen_then_assert::RULE.id, 3, 15, note),
                    (super::super::no_widen_then_assert::RULE.id, 5, 14, None)
                ]
            );
        }
    }

    #[test]
    fn exhausted_budget_reports_each_unjudged_alias() {
        use crate::engine::run;

        let rules = [alias::<2, 0>()];
        let hits = run(
            &rules,
            [(
                "types.ts",
                b"type Clean = string;\ntype X = Clean;\ntype Y = number;\ntype Z = Y;".as_slice(),
            )],
        );
        assert_eq!(
            hits.iter()
                .map(|f| (f.rule, f.line, f.col, f.note.as_deref()))
                .collect::<Vec<_>>(),
            [
                (
                    super::super::no_unknown_type_aliases::RULE.id,
                    2,
                    6,
                    Some("type too complex to judge")
                ),
                (
                    super::super::no_unknown_type_aliases::RULE.id,
                    4,
                    6,
                    Some("type too complex to judge")
                )
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
        let mut budget = Budget::new(usize::MAX);
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

#[cfg(test)]
#[path = "../../../tests/resolution_time/allowance.rs"]
mod allowance;
