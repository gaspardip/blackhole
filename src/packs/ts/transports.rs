use super::values::global_object;
use super::{
    config_responses::transport_response,
    values::{ObjectSummary, declaration, initializer, key, object_summary, path, resolve},
};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{
            Expression, ObjectProperty, ObjectPropertyKind, Statement, TSType, TSTypeName,
            TSTypeQueryExprName,
        },
    },
    span::{GetSpan, Span},
};
use std::collections::HashSet;

fn global_qualifier(ctx: &TsCtx<'_>, name: &TSTypeName<'_>) -> bool {
    let TSTypeName::IdentifierReference(id) = name else {
        return false;
    };
    global_object(id.name.as_str()) && ctx.semantic.is_reference_to_global_variable(id)
}
fn fetch<'a>(ctx: &TsCtx<'a>, mut ty: &'a TSType<'a>) -> bool {
    let mut seen = HashSet::new();
    while seen.insert(ty.span()) {
        match ty {
            TSType::TSTypeQuery(q) => {
                return match &q.expr_name {
                    TSTypeQueryExprName::IdentifierReference(id) => {
                        if ctx.semantic.is_reference_to_global_variable(id) {
                            id.name == "fetch"
                        } else {
                            initializer(ctx, id)
                                .and_then(|e| path(ctx, e))
                                .is_some_and(|p| p == ["fetch"])
                        }
                    }
                    TSTypeQueryExprName::QualifiedName(n) => {
                        n.right.name == "fetch" && global_qualifier(ctx, &n.left)
                    }
                    _ => false,
                };
            }
            TSType::TSTypeReference(r) => {
                let TSTypeName::IdentifierReference(id) = &r.type_name else {
                    return false;
                };
                let Some(AstKind::TSTypeAliasDeclaration(a)) = declaration(ctx, id) else {
                    return false;
                };
                ty = &a.type_annotation;
            }
            _ => return false,
        }
    }
    false
}

#[derive(Clone, Copy, Default)]
pub(super) enum TransportOption<'a> {
    #[default]
    Absent,
    Unknown,
    Fetch(&'a Expression<'a>),
}
impl<'a> ObjectSummary<'a> for TransportOption<'a> {
    fn field(&mut self, ctx: &TsCtx<'a>, property: &'a ObjectProperty<'a>) {
        if key(ctx, &property.key).map_or_else(
            || transport_response(ctx, &property.value),
            |name| name == "fetch",
        ) {
            *self = Self::Fetch(&property.value);
        }
    }
    fn unreadable(&mut self) {
        *self = Self::Unknown;
    }
    fn overlay(&mut self, other: &Self) {
        if !matches!(other, Self::Absent) {
            *self = *other;
        }
    }
}
fn option<'a>(ctx: &TsCtx<'a>, initial: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match object_summary(ctx, initial, &ctx.resolution.transport_options)? {
        TransportOption::Fetch(value) => Some(value),
        TransportOption::Absent | TransportOption::Unknown => None,
    }
}

fn arguments<'a>(
    ctx: &TsCtx<'a>,
    symbol: oxc::syntax::symbol::SymbolId,
    next: &mut Vec<&'a Expression<'a>>,
) {
    let decl = ctx.semantic.symbol_declaration(symbol);
    if let Some(f) = super::values::enclosing_functions(ctx, decl.id()).find_map(|n| {
        if let AstKind::Function(f) = n.kind() {
            Some(f)
        } else {
            None
        }
    }) && let Some(index) = super::values::parameter_index(&f.params, symbol)
        && let Some(fn_id) = f.id.as_ref().and_then(|id| id.symbol_id.get())
    {
        for reference in ctx.semantic.scoping().get_resolved_references(fn_id) {
            if let AstKind::CallExpression(c) =
                ctx.semantic.nodes().parent_kind(reference.node_id())
                && c.callee.span() == ctx.semantic.nodes().get_node(reference.node_id()).span()
                && let Some(arg) = c.arguments.get(index).and_then(|a| a.as_expression())
            {
                next.push(arg);
            }
        }
    }
}
#[derive(Default)]
struct FieldPaths<'a> {
    entries: Vec<(Option<usize>, &'a str)>,
    ids: std::collections::HashMap<(Option<usize>, &'a str), usize>,
}
impl<'a> FieldPaths<'a> {
    fn push(&mut self, parent: Option<usize>, key: &'a str) -> usize {
        let field = (parent, key);
        *self.ids.entry(field).or_insert_with(|| {
            self.entries.push(field);
            self.entries.len() - 1
        })
    }
}
pub(super) fn results<'a>(ctx: &TsCtx<'a>, initial: &'a Expression<'a>) -> Vec<&'a Expression<'a>> {
    enum Step<'a> {
        Visit(&'a Expression<'a>, Option<usize>),
        Leave(Span),
    }
    let mut pending = vec![Step::Visit(initial, None)];
    let mut ancestors = HashSet::new();
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    let mut paths = FieldPaths::default();
    while let Some(step) = pending.pop() {
        ctx.resolution.record_work(1);
        let (e, fields) = match step {
            Step::Visit(e, path) => (resolve(ctx, e), path),
            Step::Leave(span) => {
                ancestors.remove(&span);
                continue;
            }
        };
        if ancestors.contains(&e.span()) || !seen.insert((e.span(), fields)) {
            continue;
        }
        ancestors.insert(e.span());
        pending.push(Step::Leave(e.span()));
        let mut next = Vec::new();
        if let Expression::Identifier(id) = e
            && let Some(symbol) = super::values::symbol(ctx, id)
        {
            let decl = ctx.semantic.symbol_declaration(symbol);
            if let AstKind::VariableDeclarator(v) = decl.kind()
                && let oxc::ast::ast::BindingPattern::ObjectPattern(obj) = &v.id
                && let Some(init) = &v.init
                && let Some(k) =
                    super::values::destructured(ctx, obj, |id| id.symbol_id.get() == Some(symbol))
            {
                pending.push(Step::Visit(init, Some(paths.push(fields, k))));
                continue;
            }
            arguments(ctx, symbol, &mut next);
        }
        if let Some(m) = e.get_member_expr()
            && let Some(k) = super::values::MemberKeys(ctx).read(m)
        {
            pending.push(Step::Visit(m.object(), Some(paths.push(fields, k))));
            continue;
        }
        if let Expression::ObjectExpression(o) = e
            && let Some((parent, wanted)) = fields.map(|id| paths.entries[id])
        {
            for p in &o.properties {
                ctx.resolution.record_work(1);
                match p {
                    ObjectPropertyKind::SpreadProperty(s) => {
                        pending.push(Step::Visit(&s.argument, fields));
                    }
                    ObjectPropertyKind::ObjectProperty(p) if key(ctx, &p.key) == Some(wanted) => {
                        pending.push(Step::Visit(&p.value, parent));
                    }
                    ObjectPropertyKind::ObjectProperty(_) => (),
                }
            }
            continue;
        }
        next.extend(super::values::alternatives(e));
        if let Expression::CallExpression(c) = e {
            let callee = resolve(ctx, &c.callee);
            let body = match callee {
                Expression::Identifier(id) => match declaration(ctx, id) {
                    Some(AstKind::Function(f)) => f.body.as_deref(),
                    _ => None,
                },
                Expression::FunctionExpression(f) => f.body.as_deref(),
                Expression::ArrowFunctionExpression(f) => {
                    if let Some(value) = f.body.as_expression() {
                        pending.push(Step::Visit(value, fields));
                        continue;
                    } else if let oxc::ast::ast::ArrowFunctionBody::FunctionBody(b) = &f.body {
                        Some(&**b)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(body) = body {
                next.extend(body.statements.iter().filter_map(|s| match s {
                    Statement::ReturnStatement(r) => r.argument.as_ref(),
                    _ => None,
                }));
            }
        }
        if next.is_empty() && fields.is_none() {
            results.push(e);
        }
        pending.extend(next.into_iter().map(|e| Step::Visit(e, fields)));
    }
    results
}

pub(super) fn report<'a>(
    ctx: &TsCtx<'a>,
    kind: AstKind<'a>,
    reported: &mut HashSet<Span>,
) -> Option<usize> {
    let transport = match kind {
        AstKind::VariableDeclarator(v)
            if v.type_annotation
                .as_ref()
                .is_some_and(|a| fetch(ctx, &a.type_annotation)) =>
        {
            v.init.as_ref()?
        }
        AstKind::NewExpression(n)
            if path(ctx, &n.callee).is_some_and(|p| {
                p == ["@knocklabs/node", "default"] || p == ["@knocklabs/node", "Knock"]
            }) =>
        {
            results(ctx, n.arguments.first()?.as_expression()?)
                .into_iter()
                .filter_map(|e| option(ctx, e))
                .find(|e| transport_response(ctx, e))?
        }
        _ => return None,
    };
    (transport_response(ctx, transport) && reported.insert(resolve(ctx, transport).span()))
        .then_some(transport.span().start as usize)
}
