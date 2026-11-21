use super::values::Imported;
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, ObjectPropertyKind},
};
use oxc::span::GetSpan;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-test-doubles",
    family: Family::Tests,
    provenance: "Ports of landmines testing/no-test-doubles and anti-slop no-module-mocking. Spy-only module mocks follow landmines as observation. Unlike upstream case 303, passing vi.mock and vi to a shadowed Reflect.apply reports both uses.",
    fix: "Exercise the real implementation. Intercept network requests with MSW and use spies only to observe calls.",
    run: Run::Ts(check),
};

fn worker_entry<'a>(ctx: &TsCtx<'a>, receiver: &'a Expression<'a>) -> bool {
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

fn hono<'a>(ctx: &TsCtx<'a>, initial: &'a Expression<'a>) -> bool {
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
            if let Some(c) = super::values::call_expr(e) {
                if let Some(m) = super::values::invoked(ctx, c).callee.get_member_expr()
                    && matches!(
                        super::values::MemberKeys(ctx).read(m),
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

                continue;
            }
            match e {
                Expression::NewExpression(n)
                    if super::values::path(ctx, &n.callee).is_some_and(|p| {
                        p == ["hono", "Hono"] || p == ["@hono/zod-openapi", "OpenAPIHono"]
                    }) =>
                {
                    return true;
                }

                _ => (),
            }
        }
    }
    false
}
fn check_bindings<'a>(
    ctx: &TsCtx<'a>,
    value: &'a Expression<'a>,
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
                        if super::values::key(ctx, &p.key).is_none_or(|k| {
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
fn runner_module(source: &str) -> bool {
    matches!(
        source,
        "vitest" | "@jest/globals" | "node:test" | "@playwright/test"
    ) || source.starts_with("vitest/")
}
fn runner_import<'a>(ctx: &TsCtx<'a>, kind: AstKind<'a>) -> bool {
    use oxc::ast::ast::ImportDeclarationSpecifier;
    match kind {
        AstKind::ImportDeclaration(import) => !import.import_kind.is_type() && import.specifiers.as_ref().is_none_or(|specs| specs.is_empty() || specs.iter().any(|spec| !matches!(spec,ImportDeclarationSpecifier::ImportSpecifier(s) if s.import_kind.is_type()))) && import.source.value.as_str().is_some_and(runner_module),
        AstKind::CallExpression(call) => super::values::required_module(ctx,call).is_some_and(runner_module),
        AstKind::ImportExpression(import) => super::values::literal(ctx,&import.source).is_some_and(runner_module),
        _=>false,
    }
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let test_code = super::test_context::test_file(ctx.path)
        || ctx
            .semantic
            .nodes()
            .iter()
            .any(|n| runner_import(ctx, n.kind()));
    super::values::tracked_uses::report(ctx, RULE.id, findings);
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
        let Some(call) = super::values::site(ctx, node.kind()) else {
            continue;
        };
        let invocation = call.call;
        if super::double_mutations::framework_call(ctx, &invocation) {
            findings.report(
                match node.kind() {
                    AstKind::CallExpression(c) => super::values::call_position(ctx, c),
                    _ => call.callee.span().start as usize,
                },
                None,
            );
            continue;
        }
        if test_code
            && (super::bindings::mutation(ctx, &invocation)
                || super::double_mutations::call_writes(ctx, &invocation))
        {
            findings.report(call.span.start as usize, None);
            continue;
        }
        let Some((object, method)) = super::values::receiver(ctx, invocation.callee) else {
            continue;
        };
        if test_code
            && method == "fulfill"
            && let Some(arg) = invocation.arg(0)
            && super::playwright::canned(ctx, object, arg)
        {
            findings.report(call.span.start as usize, None);
        }
        if method == "request"
            && test_code
            && hono(ctx, object)
            && let Some(value) = invocation.arg(2)
        {
            check_bindings(ctx, value, &mut reported, findings);
        }
    }
}
