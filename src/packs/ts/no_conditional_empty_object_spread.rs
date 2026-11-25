use super::{
    local_alias::{is_local, is_type_exported},
    values::{const_initializer, invoked, outer, selector_key, symbol, unwrapped},
};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, MemberExpression, ObjectPropertyKind},
};
use oxc::{
    span::{GetSpan, Span},
    syntax::{node::NodeId, operator::UnaryOperator, symbol::SymbolId},
};
use std::collections::HashMap;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-conditional-empty-object-spread",
    family: Family::Slop,
    provenance: "Port of anti-slop no-conditional-empty-object-spread.",
    fix: "Build the object first and add the optional property only when it is present.",
    run: Run::TsConservative(check),
};

#[derive(Clone, Copy)]
enum Destination {
    Harmless,
    Alias(SymbolId),
}

type Climbs = HashMap<NodeId, Option<Destination>>;

fn is_spread_alias(ctx: &TsCtx<'_>, symbol: SymbolId) -> bool {
    // A local type export can keep an empty spread alias observable.
    is_local(ctx, symbol) && !is_type_exported(ctx, symbol)
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
                if seq
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

fn immediate_call<'a>(ctx: &TsCtx<'a>, function: &oxc::semantic::AstNode<'a>) -> bool {
    let mut id = function.id();
    let callee = function.span();
    while let Some(parent) = outer(ctx, id) {
        match parent {
            AstKind::CallExpression(call) => {
                if invoked(ctx, call).callee.span() == callee
                    || unknown_function_method(ctx, &call.callee, callee)
                {
                    return true;
                }
            }
            AstKind::SequenceExpression(_)
            | AstKind::StaticMemberExpression(_)
            | AstKind::ComputedMemberExpression(_)
            | AstKind::ArrayExpression(_)
            | AstKind::SpreadElement(_) => {}
            _ => return false,
        }
        id = parent.node_id();
    }
    false
}

fn unknown_function_method<'a>(
    ctx: &TsCtx<'a>,
    callee: &'a Expression<'a>,
    function: Span,
) -> bool {
    matches!(
        callee.get_member_expr(),
        Some(MemberExpression::ComputedMemberExpression(m))
            if unwrapped(&m.object).span() == function
                && selector_key(ctx, &m.expression).is_none()
    )
}

fn function_region<'a>(
    ctx: &TsCtx<'a>,
    function: &oxc::semantic::AstNode<'a>,
    child: Span,
) -> Option<NodeId> {
    let id = function.id();
    if let AstKind::Function(f) = function.kind()
        && f.generator
    {
        // A generator body runs as it is iterated, apart from the call that binds its parameters.
        if let Some(body) = f
            .body
            .as_ref()
            .filter(|body| body.span.contains_inclusive(child))
        {
            return Some(body.node_id.get());
        }
    }
    (!immediate_call(ctx, function)).then_some(id)
}

fn possible_immediate_region(ctx: &TsCtx<'_>, id: NodeId) -> Option<NodeId> {
    let nodes = ctx.semantic.nodes();
    let mut child = nodes.kind(id).span();
    let mut ancestors = nodes.ancestors(id);
    while let Some(node) = ancestors.next() {
        let once =
            |part: Option<oxc::span::Span>| part.is_some_and(|p| p.contains_inclusive(child));
        let boundary = match node.kind() {
            AstKind::Decorator(_) => {
                child = ancestors
                    .find(|n| matches!(n.kind(), AstKind::Class(_)))?
                    .span();
                continue;
            }
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_) => {
                if let Some(boundary) = function_region(ctx, node, child) {
                    return Some(boundary);
                }
                false
            }
            AstKind::PropertyDefinition(p) => {
                !p.r#static && once(p.value.as_ref().map(GetSpan::span))
            }
            AstKind::AccessorProperty(p) => {
                !p.r#static && once(p.value.as_ref().map(GetSpan::span))
            }
            _ => false,
        };
        if boundary {
            return Some(node.id());
        }
        child = node.kind().span();
    }
    None
}

fn dependencies(ctx: &TsCtx<'_>, climbs: &mut Climbs, symbol: SymbolId) -> Option<Vec<SymbolId>> {
    let decl = ctx.semantic.symbol_declaration(symbol);
    let scope = ctx.semantic.scoping();
    let mut declared = None;
    let mut aliases = Vec::new();
    for r in scope.get_resolved_reference_ids(symbol) {
        let node = scope.get_reference(*r).node_id();
        // Code that runs later may read the binding from above its declaration.
        if ctx.semantic.nodes().kind(node).span().start < decl.span().end
            && possible_immediate_region(ctx, node)
                == *declared.get_or_insert_with(|| possible_immediate_region(ctx, decl.id()))
        {
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
