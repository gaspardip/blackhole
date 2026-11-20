use super::test_context::{Runner, TestOption, enclosing_runners, runner};
use super::values::{Origin, Properties, origin_path, properties, resolve};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{AstKind, ast::Expression},
    span::GetSpan,
};
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-flake-masking",
    family: Family::Tests,
    provenance: "Port of landmines testing/no-flake-masking.",
    fix: "Fix the timing dependency or move slow setup into a suite hook. Wait for the event that makes the assertion true.",
    run: Run::Ts(check),
};
fn inspect<'a>(
    ctx: &TsCtx<'a>,
    expr: &'a Expression<'a>,
    keys: &[&str],
    findings: &mut Findings,
) -> bool {
    let Some(opts) = super::values::properties(ctx, expr) else {
        return false;
    };
    report_options(expr, opts, keys, findings);
    true
}
fn report_options(
    expr: &Expression<'_>,
    opts: Properties<'_>,
    keys: &[&str],
    findings: &mut Findings,
) {
    for (k, p) in opts.named {
        if keys.contains(&k) {
            findings.report(p.span.start as usize, None);
        }
    }
    if opts.unknown {
        findings.report(expr.get_inner_expression().span().start as usize, None);
    }
}
pub(super) enum Api {
    Slow,
    Config(&'static [&'static str]),
}

pub(super) fn api(runner: &Runner<'_>) -> Option<Api> {
    match runner.short()? {
        ["test", "slow"] => Some(Api::Slow),
        ["vi", "setConfig"] => Some(Api::Config(&["retry", "testTimeout", "hookTimeout"])),
        ["test", "describe", "configure"] => Some(Api::Config(&["retries"])),
        _ => None,
    }
}

fn config_retries<'a>(ctx: &TsCtx<'a>, root: &'a Expression<'a>, findings: &mut Findings) {
    let mut pending = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(value) = pending.pop() {
        let value = resolve(ctx, value);
        if !seen.insert(value.span()) {
            continue;
        }
        if let Expression::ArrayExpression(array) = value {
            pending.extend(
                array
                    .elements
                    .iter()
                    .filter_map(|element| element.as_expression()),
            );
        } else if let Some(config) = properties(ctx, value)
            && let Some(test) = config.named.get("test")
            && let Some(options) = properties(ctx, &test.value)
        {
            if let Some(projects) = options.named.get("projects") {
                pending.push(&projects.value);
            }
            report_options(&test.value, options, &["retry"], findings);
        } else {
            pending.extend(super::values::alternatives(value));
        }
    }
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    super::values::tracked_uses::report(ctx, RULE.id, findings);
    for n in ctx.semantic.nodes().iter() {
        match n.kind() {
            AstKind::ExportDefaultDeclaration(export)
                if super::config_responses::config(ctx.path)
                    && let Some(value) = export.declaration.as_expression() =>
            {
                config_retries(ctx, value, findings);
            }
            kind if let Some(c) = super::values::site(ctx, kind) => {
                let call = c.call;
                if let Some(arg) = call.arg(0)
                    && origin_path(ctx, call.callee).is_some_and(|(origin, path)| {
                        matches!(
                            (origin, path.as_slice()),
                            (Origin::Global, ["defineConfig"])
                                | (
                                    Origin::Module | Origin::Required,
                                    ["vitest/config", "defineConfig"]
                                )
                        )
                    })
                {
                    config_retries(ctx, arg, findings);
                }
                let p = runner(ctx, &call);
                match api(&p) {
                    Some(Api::Config(keys)) => {
                        if let Some(arg) = call.arg(0) {
                            if !inspect(ctx, arg, keys, findings) {
                                findings.report(arg.span().start as usize, None);
                            }
                        } else if let Some(spread) = call.unreadable_args {
                            findings.report(spread.start as usize, None);
                        }
                    }
                    Some(Api::Slow) => {
                        let at = match kind {
                            AstKind::CallExpression(call) => {
                                super::values::call_position(ctx, call)
                            }
                            _ => c.span.start as usize,
                        };
                        findings.report(at, None);
                    }
                    None if p.is_playwright()
                        && p == ["test", "setTimeout"]
                        && enclosing_runners(ctx, n.id()).next().is_some_and(|owner| {
                            owner.is_playwright() && owner.test() && owner.hook().is_none()
                        }) =>
                    {
                        findings.report(c.span.start as usize, None);
                    }
                    None if p.test_or_suite()
                        && (super::values::call_expr(call.callee).is_some()
                            || !matches!(
                                p.last,
                                Some("each" | "for" | "extend" | "scoped" | "override")
                            )) =>
                    {
                        for slot in p.test_options(ctx, &call).into_iter().flatten() {
                            match slot {
                                TestOption::Object(arg, props) => {
                                    report_options(arg, props, &["retry", "timeout"], findings);
                                }
                                TestOption::Opaque {
                                    at,
                                    before_callback: true,
                                }
                                | TestOption::Timeout(at) => findings.report(at, None),
                                TestOption::Opaque { .. } => (),
                            }
                        }
                    }
                    None => (),
                }
            }
            _ => (),
        }
    }
}
