use super::values::MemberKeys;
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

use oxc::ast::ast::{ArrowFunctionBody, Expression, FunctionBody, Statement};

pub(super) fn root<'a>(mut e: &'a Expression<'a>) -> Option<&'a str> {
    loop {
        e = e.get_inner_expression();
        if let Some(m) = e.get_member_expr() {
            e = m.object();
            continue;
        }
        match e {
            Expression::CallExpression(c) => e = &c.callee,
            Expression::TaggedTemplateExpression(t) => e = &t.tag,
            Expression::Identifier(id) => return Some(id.name.as_str()),
            _ => return None,
        }
    }
}
pub(super) fn is_test(name: &str) -> bool {
    matches!(name, "it" | "test" | "specify")
}
pub(super) fn static_method(e: &Expression<'_>, names: &[&str]) -> bool {
    e.get_member_expr()
        .and_then(|m| MemberKeys::Literal.read(m))
        .is_some_and(|key| names.contains(&key))
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

pub(super) fn chain_property(
    ctx: &TsCtx<'_>,
    e: &Expression<'_>,
    wanted: impl Fn(&str) -> bool,
) -> Option<usize> {
    let mut current = e;
    loop {
        current = resolve(ctx, current);
        if let Some(m) = current.get_member_expr() {
            if MemberKeys::Resolve(ctx).read(m).is_some_and(&wanted) {
                return Some(super::values::member_span(ctx, m).start as usize);
            }
            current = m.object();
            continue;
        }
        match current {
            Expression::CallExpression(c) => current = &c.callee,
            Expression::TaggedTemplateExpression(t) => current = &t.tag,
            _ => return None,
        }
    }
}

use super::values::{Invocation, call_expr, invoked, path, resolve, unqualified};
use crate::engine::TsCtx;
use oxc::span::GetSpan;

pub(super) fn runner<'a>(ctx: &TsCtx<'a>, call: &Invocation<'a>) -> Vec<&'a str> {
    let mut e = call.callee;
    let mut suffix = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        e = resolve(ctx, e);
        if !seen.insert(e.span()) {
            return Vec::new();
        }
        if let Some(parts) = path(ctx, e) {
            let mut parts = match unqualified(&parts) {
                ["node:test", "default", rest @ ..] => {
                    let mut p = vec!["test"];
                    p.extend(rest);
                    p
                }
                [
                    "vitest" | "@jest/globals" | "node:test" | "@playwright/test",
                    rest @ ..,
                ] => rest.to_vec(),
                other => other.to_vec(),
            };
            parts.extend(suffix.into_iter().rev());
            return parts;
        }
        if let Some(m) = e.get_member_expr()
            && let Some(k) = MemberKeys::Resolve(ctx).read(m)
        {
            suffix.push(k);
            e = m.object();
            continue;
        }
        if let Some(c) = call_expr(e) {
            e = invoked(ctx, c).callee;
            continue;
        }
        if let Expression::TaggedTemplateExpression(t) = e {
            e = &t.tag;
            continue;
        }
        return Vec::new();
    }
}
