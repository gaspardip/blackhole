use super::values::Imported;
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, IdentifierReference, ObjectPropertyKind},
};
use oxc::span::GetSpan;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-test-doubles",
    family: Family::Tests,
    provenance: "Ports of landmines testing/no-test-doubles and anti-slop no-module-mocking.",
    fix: "Exercise the real implementation. Intercept network requests with MSW and use spies only to observe calls.",
    run: Run::Ts(check),
};

fn framework(ctx: &TsCtx<'_>, id: &IdentifierReference<'_>) -> bool {
    if id.reference_id.get().is_none() {
        return false;
    }
    if super::values::symbol(ctx, id).is_none() {
        return matches!(id.name.as_str(), "vi" | "jest");
    }
    matches!(
        super::values::import_of(ctx, id),
        Some(("vitest", Imported::Named("vi")) | ("@jest/globals", Imported::Named("jest")))
    )
}

fn worker_entry(ctx: &TsCtx<'_>, receiver: &Expression<'_>) -> bool {
    let Expression::Identifier(id) = receiver else {
        return false;
    };
    let Some((path, Imported::Default)) = super::values::import_of(ctx, id) else {
        return false;
    };
    let Some(relative) = path.strip_prefix("../") else {
        return false;
    };
    let entry = relative.trim_start_matches("../");
    if !(matches!(entry, "src" | "src/index")
        || entry
            .strip_prefix("src/index.")
            .is_some_and(|ext| matches!(ext, "js" | "cjs" | "mjs" | "ts" | "cts" | "mts")))
    {
        return false;
    }
    let parts: Vec<_> = ctx.path.split(['/', '\\']).collect();
    parts.windows(5).any(|parts| {
        parts[0] == "src"
            && parts[1] == "workers"
            && matches!(parts[3], "tests" | "test" | "__tests__")
    })
}

fn hono(ctx: &TsCtx<'_>, initial: &Expression<'_>) -> bool {
    let mut pending = vec![initial];
    let mut seen = std::collections::HashSet::new();
    while let Some(e) = pending.pop() {
        for e in super::transports::results(ctx, e) {
            if !seen.insert(e.span()) {
                continue;
            }
            if worker_entry(ctx, e) {
                return true;
            }
            match e {
                Expression::NewExpression(n)
                    if super::values::path(ctx, &n.callee).is_some_and(|p| {
                        p == ["hono", "Hono"] || p == ["@hono/zod-openapi", "OpenAPIHono"]
                    }) =>
                {
                    return true;
                }
                Expression::CallExpression(c) => {
                    if let Some(m) = super::double_mutations::invoked(ctx, c)
                        .callee
                        .as_member_expression()
                        && matches!(
                            super::values::member_key(ctx, m),
                            Some(
                                "use"
                                    | "route"
                                    | "get"
                                    | "post"
                                    | "put"
                                    | "patch"
                                    | "delete"
                                    | "all"
                                    | "on"
                                    | "basePath"
                                    | "notFound"
                                    | "onError"
                            )
                        )
                    {
                        pending.push(m.object());
                    }
                }
                _ => (),
            }
        }
    }
    false
}
fn check_bindings(
    ctx: &TsCtx<'_>,
    value: &Expression<'_>,
    reported: &mut std::collections::HashSet<oxc::span::Span>,
    findings: &mut Findings,
) {
    let mut pending = vec![value];
    let mut seen = std::collections::HashSet::new();
    while let Some(value) = pending.pop() {
        for e in super::transports::results(ctx, value) {
            let Expression::ObjectExpression(o) = e else {
                continue;
            };
            if !seen.insert(o.span) {
                continue;
            }
            for p in &o.properties {
                match p {
                    ObjectPropertyKind::SpreadProperty(s) => pending.push(&s.argument),
                    ObjectPropertyKind::ObjectProperty(p)
                        if super::values::key(ctx, &p.key, p.computed).is_some_and(|k| {
                            k.starts_with(|c: char| c.is_ascii_uppercase())
                                && k.bytes().all(|b| {
                                    b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
                                })
                        }) && super::bindings::contains_function(ctx, &p.value) =>
                    {
                        if reported.insert(o.span) {
                            findings.report(o.span.start as usize, None);
                        }
                        break;
                    }
                    ObjectPropertyKind::ObjectProperty(_) => (),
                }
            }
        }
    }
}
fn runner_import(statement: &oxc::ast::ast::Statement<'_>) -> bool {
    let oxc::ast::ast::Statement::ImportDeclaration(import) = statement else {
        return false;
    };
    import.source.value.as_str().is_some_and(|source| {
        matches!(source, "vitest" | "@jest/globals" | "node:test") || source.starts_with("vitest/")
    })
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let test_code =
        super::test_context::test_file(ctx.path) || ctx.program.body.iter().any(runner_import);
    if test_code {
        super::saved_methods::check(ctx, findings);
    }
    let mut reported = std::collections::HashSet::new();
    let config = super::config_responses::config(ctx.path);
    for node in ctx.semantic.nodes().iter() {
        if test_code && let Some(at) = super::transports::report(ctx, node.kind(), &mut reported) {
            findings.report(at, None);
        }
        if test_code
            && let AstKind::ObjectExpression(obj) = node.kind()
            && super::bindings::spread(ctx, obj)
            && reported.insert(obj.span)
        {
            findings.report(obj.span.start as usize, None);
        }
        if test_code
            && (super::bindings::assignment(ctx, node.kind())
                || super::double_mutations::writes(ctx, node.kind()))
        {
            findings.report(node.span().start as usize, None);
        }
        if config
            && let AstKind::ObjectProperty(property) = node.kind()
            && super::config_responses::replacement(ctx, property)
        {
            findings.report(property.span.start as usize, None);
        }
        let AstKind::CallExpression(call) = node.kind() else {
            continue;
        };
        let invocation = super::double_mutations::invoked(ctx, call);
        if super::double_mutations::framework_call(ctx, &invocation)
            || (test_code
                && (super::bindings::mutation(ctx, &invocation)
                    || super::double_mutations::call_writes(ctx, &invocation)))
        {
            findings.report(call.span.start as usize, None);
            continue;
        }
        if test_code
            && let Some(m) = invocation.callee.as_member_expression()
            && super::values::member_key(ctx, m) == Some("fulfill")
            && let Some(arg) = invocation.arg(0)
            && super::playwright::canned(ctx, m.object(), arg)
        {
            findings.report(call.span.start as usize, None);
        }
        let Some(member) = call.callee.as_member_expression() else {
            continue;
        };
        let Some(method) = member
            .static_property_name()
            .and_then(oxc::str::JSStr::as_str)
        else {
            continue;
        };
        if method == "request"
            && test_code
            && hono(ctx, member.object())
            && let Some(value) = invocation.arg(2)
        {
            check_bindings(ctx, value, &mut reported, findings);
        }
        if matches!(method, "mock" | "doMock" | "unstable_mockModule")
            && let Expression::Identifier(id) = member.object()
            && framework(ctx, id)
        {
            findings.report(call.span.start as usize, None);
        }
    }
}
