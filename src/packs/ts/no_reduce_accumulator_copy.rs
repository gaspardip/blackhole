use super::{
    array_methods::{ArrayEvidence, ArrayType, sole_declaration},
    function_types::direct_binding,
    regions::{region, suspension_starts},
    test_context::{callback_body, callback_kind},
    values::{
        binding_value_until, enclosing_functions, has_with, invocation_target, path_is, resolve,
        resolve_with, same_key, symbol, unwrapped,
    },
};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{
            CallExpression, Expression, IdentifierReference, VariableDeclarationKind,
            VariableDeclarator,
        },
    },
    semantic::{AstNode, AstNodes},
    span::{GetSpan, Span},
    syntax::operator::AssignmentOperator,
    syntax::{node::NodeId, scope::ScopeId, symbol::SymbolId},
};
use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-reduce-accumulator-copy",
    family: Family::Slop,
    gate: Gate::Blocking,
    provenance: "Port of anti-slop no-reduce-accumulator-copy and oxc no-accumulating-spread.",
    fix: "Mutate and return a fresh, locally owned accumulator, or use a single transformation to build the result.",
    run: Run::TsConservative(check),
};

#[derive(Clone, Copy)]
struct Reducer {
    accumulator: SymbolId,
    array: bool,
    body: Span,
    // A read of the accumulator that ends by here sees the incoming value.
    incoming_until: u32,
}

impl Reducer {
    fn is_accumulator<'a>(self, ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> bool {
        let stop = Some(self.accumulator);
        // `reduce` passes the accumulator, so a default chosen by a self-call never applies to it.
        let value = resolve_with(ctx, expr, |ctx, e| binding_value_until(ctx, e, stop));
        matches!(value, Expression::Identifier(id)
            if symbol(ctx, id) == stop && id.span.end <= self.incoming_until)
    }
}

// Where the first plain `acc = ...` statement of the callback ends, or `None` when any other
// write, or one in a nested function or a loop, can replace the accumulator.
fn incoming_until(ctx: &TsCtx<'_>, callback: NodeId, accumulator: SymbolId) -> Option<u32> {
    let (scoping, nodes) = (ctx.semantic.scoping(), ctx.semantic.nodes());
    let mut until = u32::MAX;
    for reference in scoping.get_resolved_references(accumulator) {
        if !ctx.resolution.take_work(1) {
            return None;
        }
        if !reference.is_write() {
            continue;
        }
        let at = reference.node_id();
        let target = nodes.kind(at).span();
        let assignment = nodes
            .ancestors(at)
            .take_while(|_| ctx.resolution.take_work(1))
            .find_map(|n| n.kind().as_assignment_expression())
            .filter(|e| e.operator == AssignmentOperator::Assign && e.left.span() == target)?;
        let statement = matches!(
            nodes.parent_kind(assignment.node_id.get()),
            AstKind::ExpressionStatement(_)
        );
        let nested = nodes
            .ancestors(at)
            .take_while(|n| n.id() != callback && ctx.resolution.take_work(1))
            .any(|n| {
                n.kind().is_iteration_statement()
                    || matches!(
                        n.kind(),
                        AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
                    )
            });
        if !statement || nested {
            return None;
        }
        until = until.min(assignment.span.end);
    }
    Some(until)
}

fn reducer<'a>(
    ctx: &TsCtx<'a>,
    node: &'a CallExpression<'a>,
    evidence: &mut ArrayEvidence,
    incoming: &mut HashMap<SymbolId, Option<u32>>,
) -> Option<(NodeId, Reducer)> {
    let target = invocation_target(ctx, node)?;
    target.method(
        ctx,
        |name| same_key(ctx, name, "reduce") || same_key(ctx, name, "reduceRight"),
        |e| evidence.known(ctx, e),
    )?;
    let call = target.expand(ctx, false);
    if call.args.len() > 2 {
        return None;
    }
    let callback = callback_kind(ctx, resolve(ctx, call.arg(0)?))?;
    let params = match callback {
        AstKind::Function(f) if !f.generator => &f.params,
        AstKind::ArrowFunctionExpression(f) => &f.params,
        _ => return None,
    };
    let function = callback.node_id();
    let body = callback_body(callback)?;
    let first = params.items.first()?;
    let accumulator = first.pattern.get_binding_identifier()?.symbol_id.get()?;
    sole_declaration(ctx, accumulator)?;
    let array = call.arg(1).is_some_and(|e| evidence.known(ctx, e))
        || first.type_annotation.as_ref().is_some_and(|a| {
            matches!(
                evidence.array_type(ctx, &a.type_annotation),
                ArrayType::Array
            )
        });
    let incoming_until = (*incoming
        .entry(accumulator)
        .or_insert_with(|| incoming_until(ctx, function, accumulator)))?;
    Some((
        function,
        Reducer {
            accumulator,
            array,
            body,
            incoming_until,
        },
    ))
}

fn iteration_binding(nodes: &AstNodes<'_>, statement: NodeId) -> bool {
    let span = nodes.get_node(statement).span();
    match nodes.parent_kind(statement) {
        AstKind::ForInStatement(loop_) => loop_.left.span() == span,
        AstKind::ForOfStatement(loop_) => loop_.left.span() == span,
        _ => false,
    }
}

fn target_declarator<'a>(ctx: &TsCtx<'a>, symbol: SymbolId) -> Option<&'a VariableDeclarator<'a>> {
    let redeclarations = ctx.semantic.scoping().symbol_redeclarations(symbol);
    if let Some(kind) = sole_declaration(ctx, symbol) {
        return kind.as_variable_declarator();
    }
    let nodes = ctx.semantic.nodes();
    let mut initialized = None;
    for redeclaration in redeclarations {
        if !ctx.resolution.take_work(1) {
            return None;
        }
        // A name shared with a type is ambiguous; `as_variable_declarator` rejects it.
        let decl = nodes
            .kind(redeclaration.declaration)
            .as_variable_declarator()?;
        let statement = nodes.parent_node(decl.node_id.get());
        if !matches!(statement.kind(), AstKind::VariableDeclaration(v) if v.kind == VariableDeclarationKind::Var)
            || iteration_binding(nodes, statement.id())
        {
            return None;
        }
        if decl.init.is_some() && initialized.replace(decl).is_some() {
            return None;
        }
    }
    initialized
}

#[derive(Default)]
struct FreshTargets<'a> {
    eval_scopes: HashSet<ScopeId>,
    block_functions: OnceCell<HashMap<ScopeId, Vec<&'a str>>>,
}

impl<'a> FreshTargets<'a> {
    fn has_write(&self, ctx: &TsCtx<'_>, symbol: SymbolId) -> bool {
        let scoping = ctx.semantic.scoping();
        scoping
            .get_resolved_references(symbol)
            .any(|reference| !ctx.resolution.take_work(1) || reference.is_write())
            || (!scoping.symbol_flags(symbol).is_const_variable()
                && self.eval_scopes.contains(&scoping.symbol_scope_id(symbol)))
    }

    fn has_block_function(&self, ctx: &TsCtx<'a>, owner: ScopeId, name: &str) -> bool {
        self.block_functions
            .get_or_init(|| {
                let nodes = ctx.semantic.nodes();
                let scoping = ctx.semantic.scoping();
                let mut by_scope: HashMap<_, Vec<_>> = HashMap::new();
                for node in nodes.iter().take_while(|_| ctx.resolution.take_work(1)) {
                    if let AstKind::Function(f) = node.kind()
                        && f.is_function_declaration()
                        && let Some(id) = &f.id
                        && !matches!(nodes.parent_kind(node.id()), AstKind::FunctionBody(_))
                        && let Some(scope) = scoping
                            .scope_ancestors(node.scope_id())
                            .take_while(|_| ctx.resolution.take_work(1))
                            .find(|&scope| scoping.scope_flags(scope).is_var())
                    {
                        by_scope.entry(scope).or_default().push(id.name.as_str());
                    }
                }
                by_scope
            })
            .get(&owner)
            .is_some_and(|names| {
                names
                    .iter()
                    .take_while(|_| ctx.resolution.take_work(1))
                    .any(|other| same_key(ctx, name, other))
            })
    }
}

fn declared_fresh<'a>(
    ctx: &TsCtx<'a>,
    id: &IdentifierReference<'_>,
    targets: &FreshTargets<'a>,
) -> Option<&'a Expression<'a>> {
    let symbol = symbol(ctx, id)?;
    if targets.has_write(ctx, symbol) {
        return None;
    }
    let decl = target_declarator(ctx, symbol)?;
    let init = unwrapped(
        ctx,
        direct_binding(AstKind::VariableDeclarator(decl))?.initializer?,
    );
    if decl.span.end > id.span.start
        || !matches!(
            init,
            Expression::ObjectExpression(_) | Expression::ArrayExpression(_)
        )
    {
        return None;
    }
    let nodes = ctx.semantic.nodes();
    let statement = nodes.parent_node(decl.node_id.get());
    let AstKind::VariableDeclaration(declaration) = statement.kind() else {
        return None;
    };
    if matches!(
        declaration.kind,
        VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
    ) || iteration_binding(nodes, statement.id())
    {
        return None;
    }
    let statement = statement.id();
    // The read must run after the initializer: same function, inside the declaration's own block.
    let container = nodes
        .ancestors(statement)
        .take_while(|_| ctx.resolution.take_work(1))
        .find(|node| !matches!(node.kind(), AstKind::LabeledStatement(_)))?;
    let conditional_body = match container.kind() {
        AstKind::IfStatement(_)
        | AstKind::WhileStatement(_)
        | AstKind::DoWhileStatement(_)
        | AstKind::ForInStatement(_)
        | AstKind::ForOfStatement(_) => true,
        AstKind::ForStatement(loop_) => loop_.body.span().contains_inclusive(decl.span),
        _ => false,
    };
    let container = if conditional_body {
        nodes.get_node(statement)
    } else {
        container
    };
    let function = enclosing_functions(ctx, statement).next().map(AstNode::id);
    let mut same_block = false;
    let read_function = nodes
        .ancestors(id.node_id.get())
        .take_while(|_| ctx.resolution.take_work(1))
        .find(|node| {
            same_block |= node.id() == container.id();
            matches!(
                node.kind(),
                AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
            )
        });
    if !same_block || function != read_function.map(AstNode::id) {
        return None;
    }
    // Annex B can copy a nested function declaration into the surrounding var binding.
    if declaration.kind == VariableDeclarationKind::Var
        && targets.has_block_function(
            ctx,
            ctx.semantic.scoping().symbol_scope_id(symbol),
            id.name.as_str(),
        )
    {
        return None;
    }
    Some(init)
}

fn fresh_target<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
    callback: NodeId,
    targets: &FreshTargets<'a>,
    regions: &mut HashMap<NodeId, Option<NodeId>>,
) -> bool {
    // Only the target as written may name a fresh local; one reached through a member is an alias.
    let target = match unwrapped(ctx, expr) {
        Expression::Identifier(id) => declared_fresh(ctx, id, targets),
        value => Some(resolve_with(ctx, value, unwrapped)),
    };
    let Some(target) = target else { return false };
    let id = match target {
        Expression::ObjectExpression(object) => object.node_id.get(),
        Expression::ArrayExpression(array) => array.node_id.get(),
        _ => return false,
    };
    region(ctx, regions, id) == Some(callback)
}

fn copies<'a>(
    ctx: &TsCtx<'a>,
    node: &'a CallExpression<'a>,
    callback: NodeId,
    reducer: Reducer,
    evidence: &mut ArrayEvidence,
    targets: &FreshTargets<'a>,
    regions: &mut HashMap<NodeId, Option<NodeId>>,
) -> bool {
    let Some(target) = invocation_target(ctx, node) else {
        return false;
    };
    let accumulator = |e: &'a Expression<'a>| reducer.is_accumulator(ctx, e);
    match target.global_path() {
        Some(path) if path_is(ctx, path, &["Object", "assign"]) => {
            let call = target.expand(ctx, false);
            call.arg(0)
                .is_some_and(|e| fresh_target(ctx, e, callback, targets, regions))
                && call
                    .positional()
                    .iter()
                    .skip(1)
                    .flatten()
                    .any(|e| accumulator(e))
        }
        Some(path) if path_is(ctx, path, &["Array", "from"]) => {
            reducer.array && target.expand(ctx, false).arg(0).is_some_and(accumulator)
        }
        _ => {
            reducer.array
                && target
                    .method(
                        ctx,
                        |name| {
                            [
                                "concat",
                                "slice",
                                "toSpliced",
                                "toSorted",
                                "toReversed",
                                "with",
                            ]
                            .into_iter()
                            .any(|method| same_key(ctx, name, method))
                        },
                        |e| accumulator(e) || evidence.known(ctx, e),
                    )
                    .is_some_and(accumulator)
        }
    }
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    if has_with(ctx) {
        return;
    }
    let nodes = ctx.semantic.nodes();
    ctx.resolution.limit_work(nodes.len().saturating_mul(40));
    let mut evidence = ArrayEvidence::default();
    let mut reducers = HashMap::new();
    let mut incoming = HashMap::new();
    let mut targets = FreshTargets::default();
    let scoping = ctx.semantic.scoping();
    for node in nodes.iter() {
        if !ctx.resolution.take_work(1) {
            return;
        }
        let AstKind::CallExpression(c) = node.kind() else {
            continue;
        };
        // TypeScript's default emit can make an optional eval call direct.
        if let Expression::Identifier(id) = super::values::inner(ctx, &c.callee)
            && same_key(ctx, id.name.as_str(), "eval")
        {
            for scope in scoping.scope_ancestors(node.scope_id()) {
                if !ctx.resolution.take_work(1) || !targets.eval_scopes.insert(scope) {
                    break;
                }
            }
        }
        if let Some((callback, context)) = reducer(ctx, c, &mut evidence, &mut incoming) {
            reducers
                .entry(callback)
                .and_modify(|r: &mut Reducer| r.array |= context.array)
                .or_insert(context);
        }
    }
    if reducers.is_empty() {
        return;
    }
    let suspensions = suspension_starts(ctx);
    let mut regions = HashMap::new();
    let body_span = reducers
        .values()
        .map(|reducer| reducer.body)
        .reduce(|left, right| Span::new(left.start.min(right.start), left.end.max(right.end)))
        .expect("at least one reducer");
    let reducer_of = |id, regions: &mut HashMap<_, _>| {
        let copy = nodes.kind(id).span();
        if !body_span.contains_inclusive(copy) {
            return None;
        }
        let callback = region(ctx, regions, id)?;
        let reducer = *reducers.get(&callback)?;
        if !reducer.body.contains_inclusive(copy) || copy.end > reducer.incoming_until {
            return None;
        }
        if !ctx
            .resolution
            .take_work(suspensions.len().bit_width() as usize)
        {
            return None;
        }
        // A copy after a suspension may run after the callback returned.
        let start = nodes.kind(callback).span().start;
        let first = suspensions.partition_point(|&at| at < start);
        suspensions
            .get(first)
            .is_none_or(|&at| at >= copy.start)
            .then_some((callback, reducer))
    };
    for (id, node) in nodes.iter_enumerated() {
        if !ctx.resolution.take_work(1) {
            break;
        }
        let copying = match node.kind() {
            AstKind::CallExpression(c) => {
                reducer_of(id, &mut regions).is_some_and(|(span, context)| {
                    copies(ctx, c, span, context, &mut evidence, &targets, &mut regions)
                })
            }
            AstKind::SpreadElement(s) => {
                matches!(
                    nodes.parent_kind(id),
                    AstKind::ArrayExpression(_) | AstKind::ObjectExpression(_)
                ) && reducer_of(id, &mut regions)
                    .is_some_and(|(_, context)| context.is_accumulator(ctx, &s.argument))
            }
            _ => continue,
        };
        if copying && ctx.resolution.has_work() {
            findings.report(node.span().start as usize, None);
        }
    }
}
