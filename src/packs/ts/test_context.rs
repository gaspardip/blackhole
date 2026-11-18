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
pub(super) fn member(e: &Expression<'_>, receiver: &str, key: &str) -> bool {
    matches!(e,Expression::StaticMemberExpression(m) if m.property.name==key && matches!(&m.object,Expression::Identifier(id) if id.name==receiver))
}
fn line<'a>(body: &'a FunctionBody<'a>, allow_return: bool) -> Option<&'a Expression<'a>> {
    if !body.directives.is_empty() {
        return None;
    }
    match body.statements.as_slice() {
        [Statement::ExpressionStatement(s)] => Some(&s.expression),
        [Statement::ReturnStatement(s)] if allow_return => s.argument.as_ref(),
        _ => None,
    }
}
pub(super) fn single<'a>(
    callback: &'a Expression<'a>,
    allow_return: bool,
) -> Option<&'a Expression<'a>> {
    match callback {
        Expression::FunctionExpression(f) => line(f.body.as_ref()?, allow_return),
        Expression::ArrowFunctionExpression(f) => match &f.body {
            ArrowFunctionBody::FunctionBody(b) => line(b, allow_return),
            b => b.as_expression(),
        },
        _ => None,
    }
}
