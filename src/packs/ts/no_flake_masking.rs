use super::test_context::{TestOption, runner};
use super::values::Properties;
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
const CONFIG_OPTIONS: &[(&[&str], &[&str])] = &[
    (
        &["vi", "setConfig"],
        &["retry", "testTimeout", "hookTimeout"],
    ),
    (&["test", "describe", "configure"], &["retries"]),
];
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    super::values::tracked_uses::report(ctx, RULE.id, findings);
    for n in ctx.semantic.nodes().iter() {
        match n.kind() {
            AstKind::ObjectProperty(p) if super::values::key(ctx, &p.key) == Some("test") => {
                inspect(ctx, &p.value, &["retry"], findings);
            }
            kind if let Some(c) = super::values::site(ctx, kind) => {
                let call = c.call;
                let p = runner(ctx, &call);
                if let Some((_, keys)) = CONFIG_OPTIONS
                    .iter()
                    .find(|(path, _)| p.short() == Some(*path))
                {
                    if let Some(arg) = call.arg(0)
                        && !inspect(ctx, arg, keys, findings)
                    {
                        findings.report(arg.span().start as usize, None);
                    }
                    continue;
                }
                match p.short() {
                    Some(["test", "slow"]) => {
                        let at = match kind {
                            AstKind::CallExpression(call) => {
                                super::values::call_position(ctx, call)
                            }
                            _ => c.span.start as usize,
                        };
                        findings.report(at, None);
                    }
                    _ if p.test_or_suite()
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
                    _ => (),
                }
            }
            _ => (),
        }
    }
}
