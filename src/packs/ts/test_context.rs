use super::values::MemberKeys;
use super::values::{Invocation, Origin, call_expr, invoked, origin_path, resolve};
use crate::engine::TsCtx;
use oxc::ast::ast::{ArrowFunctionBody, Expression, FunctionBody, Statement};
use oxc::span::GetSpan;

pub(super) fn test_file(path: &str) -> bool {
    let mut parts = path.rsplit(['/', '\\']);
    let file = parts.next().unwrap_or_default();
    file.split('.').any(|part| {
        matches!(
            part,
            "test" | "spec" | "fixture" | "mock" | "setup" | "commands"
        )
    }) || parts.any(|part| {
        matches!(
            part,
            "__tests__" | "test" | "tests" | "testing" | "test-utils"
        )
    })
}

pub(super) fn is_test(name: &str) -> bool {
    matches!(name, "it" | "test" | "specify")
}
fn single_statement<'a>(body: &'a FunctionBody<'a>) -> Option<&'a Statement<'a>> {
    if !body.directives.is_empty() {
        return None;
    }
    let [statement] = body.statements.as_slice() else {
        return None;
    };
    Some(statement)
}
fn callback_value<'a>(
    callback: &'a Expression<'a>,
    statement: fn(&'a Statement<'a>) -> Option<&'a Expression<'a>>,
) -> Option<&'a Expression<'a>> {
    let body = match callback {
        Expression::FunctionExpression(f) => f.body.as_deref()?,
        Expression::ArrowFunctionExpression(f) => match &f.body {
            ArrowFunctionBody::FunctionBody(b) => b,
            other => return other.as_expression(),
        },
        _ => return None,
    };
    statement(single_statement(body)?)
}
fn expression_statement<'a>(s: &'a Statement<'a>) -> Option<&'a Expression<'a>> {
    match s {
        Statement::ExpressionStatement(s) => Some(&s.expression),
        _ => None,
    }
}
pub(super) fn single_effect<'a>(callback: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    callback_value(callback, expression_statement)
}
pub(super) fn single_value<'a>(callback: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    callback_value(callback, |s| match s {
        Statement::ReturnStatement(r) => r.argument.as_ref(),
        _ => expression_statement(s),
    })
}
pub(super) fn suite(name: &str) -> bool {
    matches!(name, "describe" | "suite" | "context")
}

#[derive(Clone, Copy, Default, PartialEq)]
enum ModuleRoot {
    #[default]
    None,
    DefaultTest,
    CallableTest,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Runner<'a> {
    prefix: [&'a str; 3],
    length: usize,
    module_root: ModuleRoot,
    pub(super) last: Option<&'a str>,
    pub(super) skipped: bool,
    pub(super) skip_at: Option<usize>,
    pub(super) todo: bool,
}
impl<'a> Runner<'a> {
    fn push(&mut self, name: &'a str) {
        let root = std::mem::take(&mut self.module_root);
        if root == ModuleRoot::CallableTest
            && matches!(
                name,
                "default"
                    | "test"
                    | "it"
                    | "describe"
                    | "suite"
                    | "before"
                    | "after"
                    | "beforeEach"
                    | "afterEach"
                    | "mock"
                    | "assert"
                    | "run"
            )
        {
            self.length = 0;
        }
        let name = if root != ModuleRoot::None && name == "default" {
            "test"
        } else {
            name
        };
        if let Some(slot) = self.prefix.get_mut(self.length) {
            *slot = name;
        }
        self.skipped |= self.length > 0 && super::no_skipped_tests::modifier(name);
        self.length += 1;
        self.last = Some(name);
    }
    pub(super) fn short(&self) -> Option<&[&'a str]> {
        self.prefix.get(..self.length)
    }
    pub(super) fn first(&self) -> Option<&&'a str> {
        (self.length > 0).then_some(&self.prefix[0])
    }
    pub(super) fn is_empty(&self) -> bool {
        self.length == 0
    }
}
impl<const N: usize> PartialEq<[&str; N]> for Runner<'_> {
    fn eq(&self, other: &[&str; N]) -> bool {
        self.short() == Some(other.as_slice())
    }
}

pub(super) fn runner<'a>(ctx: &TsCtx<'a>, call: &Invocation<'a>) -> Runner<'a> {
    let mut e = call.callee;
    let mut steps = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut result = loop {
        e = resolve(ctx, e);
        let span = e.span();
        if let Some(result) = ctx.resolution.runners.borrow().get(&span) {
            break *result;
        }
        if !seen.insert(span) {
            return Runner::default();
        }
        if let Some(m) = e.get_member_expr()
            && let Some(key) = MemberKeys(ctx).read(m)
        {
            let at = super::values::member_span(ctx, m).start as usize;
            steps.push((span, Some((key, at))));
            e = m.object();
            continue;
        }
        if let Some((origin, parts)) = origin_path(ctx, e) {
            let mut result = Runner::default();
            let names = match (origin, parts.as_slice()) {
                (Origin::Global, names)
                | (_, names @ ["node:timers/promises" | "timers/promises", ..]) => names,
                (Origin::Required, ["node:test", rest @ ..]) => {
                    result.push("test");
                    result.module_root = ModuleRoot::CallableTest;
                    rest
                }
                (_, ["node:test" | "@playwright/test", rest @ ..]) => {
                    result.module_root = ModuleRoot::DefaultTest;
                    rest
                }
                (_, ["vitest" | "@jest/globals", rest @ ..]) => rest,
                _ => return Runner::default(),
            };
            for name in names {
                result.push(name);
            }
            ctx.resolution.runners.borrow_mut().insert(span, result);
            break result;
        }
        if let Some(c) = call_expr(e) {
            steps.push((span, None));
            e = invoked(ctx, c).callee;
            continue;
        }
        if let Expression::TaggedTemplateExpression(t) = e {
            steps.push((span, None));
            e = &t.tag;
            continue;
        }
        return Runner::default();
    };
    let mut cache = ctx.resolution.runners.borrow_mut();
    for (span, key) in steps.into_iter().rev() {
        if let Some((key, at)) = key {
            if super::no_skipped_tests::modifier(key) {
                result.skip_at = Some(at);
            }
            result.todo |= key == "todo";
            result.push(key);
        }
        cache.insert(span, result);
    }
    result
}

impl Runner<'_> {
    pub(super) fn test(&self) -> bool {
        self.first().is_some_and(|name| is_test(name))
    }
    pub(super) fn test_or_suite(&self) -> bool {
        self.test() || self.first().is_some_and(|name| suite(name))
    }
}

impl<'a> Invocation<'a> {
    pub(super) fn test_options(&self) -> [Option<&'a Expression<'a>>; 2] {
        [self.arg(1), self.arg(2)]
    }
    pub(super) fn inline_test_body(&self) -> Option<oxc::span::Span> {
        self.args
            .iter()
            .flatten()
            .find_map(|arg| match super::values::unwrapped(arg) {
                Expression::ArrowFunctionExpression(f) => Some(f.body.span()),
                Expression::FunctionExpression(f) => Some(f.body.as_ref()?.span),
                _ => None,
            })
    }
}
pub(super) fn enclosing_runners<'ctx, 'a>(
    ctx: &'ctx TsCtx<'a>,
    id: oxc::syntax::node::NodeId,
) -> impl Iterator<Item = Runner<'a>> + 'ctx {
    super::values::enclosing_functions(ctx, id).map(|f| {
        super::values::enclosing_call(ctx, f.id())
            .map_or_else(Runner::default, |call| runner(ctx, &call))
    })
}
