use super::{
    local_alias::{is_local, is_type_exported},
    values::{const_initializer, outer, symbol, unwrapped},
};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, ObjectPropertyKind, VariableDeclarationKind},
};
use oxc::{
    span::{GetSpan, Span},
    syntax::{node::NodeId, operator::UnaryOperator, symbol::SymbolId},
};
use std::collections::HashMap;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-conditional-empty-object-spread",
    family: Family::Slop,
    gate: Gate::Blocking,
    provenance: "Port of anti-slop no-conditional-empty-object-spread.",
    fix: "Build the object first and add the optional property only when it is present.",
    run: Run::Ts(check),
};

#[derive(Clone, Copy)]
enum Destination {
    Harmless,
    Alias(SymbolId),
}

type Climbs = HashMap<NodeId, Option<Destination>>;

fn is_spread_alias(ctx: &TsCtx<'_>, symbol: SymbolId) -> bool {
    let binding = ctx.semantic.symbol_declaration(symbol);
    let AstKind::VariableDeclaration(declaration) = ctx.semantic.nodes().parent_kind(binding.id())
    else {
        return false;
    };
    // A local type export can keep an empty spread alias observable.
    declaration.kind == VariableDeclarationKind::Const
        && is_local(ctx, symbol)
        && !is_type_exported(ctx, symbol)
}

fn destination(ctx: &TsCtx<'_>, climbs: &mut Climbs, mut node: NodeId) -> Option<Destination> {
    let nodes = ctx.semantic.nodes();
    let mut path = Vec::new();
    let found = loop {
        // Arms of one nest share its way out, so each conditional is climbed once.
        if let Some(known) = climbs.get(&node) {
            break *known;
        }
        let span = nodes.kind(node).span();
        match outer(ctx, node) {
            Some(AstKind::SequenceExpression(seq))
                if !matches!(nodes.kind(node), AstKind::IdentifierReference(_))
                    && seq
                        .expressions
                        .last()
                        .is_some_and(|last| last.get_inner_expression().span() == span) =>
            {
                node = seq.node_id.get();
                path.push(node);
            }
            Some(AstKind::ConditionalExpression(c))
                if [&c.consequent, &c.alternate]
                    .iter()
                    .any(|arm| arm.get_inner_expression().span() == span) =>
            {
                node = c.node_id.get();
                path.push(node);
            }
            Some(AstKind::SpreadElement(s))
                if matches!(
                    nodes.parent_kind(s.node_id.get()),
                    AstKind::ObjectExpression(_)
                ) =>
            {
                break Some(Destination::Harmless);
            }
            Some(AstKind::UnaryExpression(u)) if u.operator == UnaryOperator::Void => {
                break Some(Destination::Harmless);
            }
            Some(AstKind::VariableDeclarator(v))
                if v.init
                    .as_ref()
                    .is_some_and(|init| init.get_inner_expression().span() == span) =>
            {
                break v
                    .id
                    .get_binding_identifier()
                    .and_then(|binding| binding.symbol_id.get())
                    .filter(|alias| is_spread_alias(ctx, *alias))
                    .map(Destination::Alias);
            }
            _ => break None,
        }
    };
    climbs.extend(path.into_iter().map(|c| (c, found)));
    found
}

fn dependencies(ctx: &TsCtx<'_>, climbs: &mut Climbs, symbol: SymbolId) -> Option<Vec<SymbolId>> {
    let declaration_end = ctx.semantic.symbol_declaration(symbol).span().end;
    let scope = ctx.semantic.scoping();
    let mut aliases = Vec::new();
    for r in scope.get_resolved_reference_ids(symbol) {
        let node = scope.get_reference(*r).node_id();
        if ctx.semantic.nodes().kind(node).span().start < declaration_end {
            return None;
        }
        if let Destination::Alias(alias) = destination(ctx, climbs, node)? {
            aliases.push(alias);
        }
    }
    Some(aliases)
}

#[derive(Default)]
struct Visibility<'a> {
    terminals: HashMap<SymbolId, Option<&'a Expression<'a>>>,
    known: HashMap<SymbolId, bool>,
    climbs: Climbs,
    empty_arms: HashMap<Span, bool>,
}

impl<'a> Visibility<'a> {
    fn harmless(&mut self, ctx: &TsCtx<'_>, symbol: SymbolId) -> bool {
        enum Visit {
            Enter(SymbolId),
            Finish(SymbolId, Vec<SymbolId>),
        }
        if let Some(&known) = self.known.get(&symbol) {
            return known;
        }
        let mut stack = vec![Visit::Enter(symbol)];
        while let Some(step) = stack.pop() {
            match step {
                Visit::Enter(id) => {
                    if self.known.contains_key(&id) {
                        continue;
                    }
                    // Cycles stay unknown until every outgoing use has been proved harmless.
                    self.known.insert(id, false);
                    if let Some(aliases) = dependencies(ctx, &mut self.climbs, id) {
                        stack.push(Visit::Finish(id, aliases.clone()));
                        stack.extend(aliases.into_iter().map(Visit::Enter));
                    }
                }
                Visit::Finish(id, aliases) => {
                    let safe = aliases.iter().all(|a| self.known[a]);
                    self.known.insert(id, safe);
                }
            }
        }
        self.known[&symbol]
    }
    fn value(&mut self, ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
        let mut expr = unwrapped(expr);
        let mut path = Vec::new();
        // The visibility check rejects alias cycles before the next initializer is followed.
        let terminal = loop {
            let Expression::Identifier(id) = expr else {
                break Some(expr);
            };
            let Some(next) = const_initializer(ctx, id) else {
                break None;
            };
            let Some(symbol) = symbol(ctx, id).filter(|&binding| is_spread_alias(ctx, binding))
            else {
                break None;
            };
            if let Some(&cached) = self.terminals.get(&symbol) {
                break cached;
            }
            path.push(symbol);
            if !self.harmless(ctx, symbol) {
                break None;
            }
            expr = unwrapped(next);
        };
        self.terminals
            .extend(path.into_iter().map(|symbol| (symbol, terminal)));
        terminal
    }

    fn contains_empty(&mut self, ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> bool {
        let mut pending = vec![(expr, false)];
        while let Some((value, expanded)) = pending.pop() {
            let span = value.span();
            if !expanded && self.empty_arms.contains_key(&span) {
                continue;
            }
            let empty = match self.value(ctx, value) {
                Some(Expression::ObjectExpression(o)) => o.properties.is_empty(),
                Some(Expression::ConditionalExpression(c)) => {
                    let arms = [&c.consequent, &c.alternate];
                    if !expanded {
                        self.empty_arms.insert(span, false);
                        pending.push((value, true));
                        pending.extend(arms.map(|arm| (arm, false)));
                        continue;
                    }
                    arms.iter().any(|arm| self.empty_arms[&arm.span()])
                }
                _ => false,
            };
            self.empty_arms.insert(span, empty);
        }
        self.empty_arms[&expr.span()]
    }
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut visibility = Visibility::default();
    for node in ctx.semantic.nodes().iter() {
        let AstKind::ObjectExpression(object) = node.kind() else {
            continue;
        };
        for property in &object.properties {
            if let ObjectPropertyKind::SpreadProperty(spread) = property
                && matches!(
                    visibility.value(ctx, &spread.argument),
                    Some(Expression::ConditionalExpression(_))
                )
                && visibility.contains_empty(ctx, &spread.argument)
            {
                findings.report(spread.span.start as usize, None);
            }
        }
    }
}
