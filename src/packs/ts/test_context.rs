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
        if let Some(m) = e.as_member_expression() {
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
pub(super) fn direct_member(e: &Expression<'_>, receiver: &str, key: &str) -> bool {
    let Expression::StaticMemberExpression(m) = e else {
        return false;
    };
    m.property.name == key && matches!(&m.object,Expression::Identifier(id) if id.name==receiver)
}
pub(super) fn static_method(e: &Expression<'_>, names: &[&str]) -> bool {
    let Expression::StaticMemberExpression(m) = e else {
        return false;
    };
    names.contains(&m.property.name.as_str())
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

pub(super) fn chain_property(e: &Expression<'_>, wanted: impl Fn(&str) -> bool) -> Option<usize> {
    let mut current = e;
    loop {
        current = current.get_inner_expression();
        if let Some(m) = current.as_member_expression() {
            if let Some((span, key)) = m.static_property_info()
                && key.as_str().is_some_and(&wanted)
            {
                return Some(span.start as usize);
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
