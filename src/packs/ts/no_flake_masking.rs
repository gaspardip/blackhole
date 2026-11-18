use super::test_context::runner;
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
    expr: Option<&'a Expression<'a>>,
    keys: &[&str],
    findings: &mut Findings,
) -> bool {
    let Some(expr) = expr else {
        return false;
    };
    let Some(opts) = super::values::properties(ctx, expr) else {
        return false;
    };
    for (k, p) in opts.named {
        if keys.contains(&k) {
            findings.report(p.span.start as usize, None);
        }
    }
    if opts.unknown {
        findings.report(expr.get_inner_expression().span().start as usize, None);
    }
    true
}
const CONFIG_OPTIONS: &[(&[&str], &[&str])] = &[
    (
        &["vi", "setConfig"],
        &["retry", "testTimeout", "hookTimeout"],
    ),
    (&["test", "describe", "configure"], &["retries"]),
];
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for n in ctx.semantic.nodes().iter() {
        match n.kind() {
            AstKind::ObjectProperty(p) if super::values::key(ctx, &p.key) == Some("test") => {
                inspect(ctx, Some(&p.value), &["retry"], findings);
            }
            AstKind::CallExpression(c) => {
                let call = super::values::invoked(ctx, c);
                let p = runner(ctx, &call);
                if let Some((_, keys)) = CONFIG_OPTIONS
                    .iter()
                    .find(|(path, _)| p.short() == Some(*path))
                {
                    if let Some(arg) = call.arg(0)
                        && !inspect(ctx, Some(arg), keys, findings)
                    {
                        findings.report(arg.span().start as usize, None);
                    }
                    continue;
                }
                match p.short() {
                    Some(["test", "slow"]) => findings.report(c.span.start as usize, None),
                    _ if p.test_or_suite()
                        && (super::values::call_expr(call.callee).is_some()
                            || !matches!(
                                p.last,
                                Some("each" | "for" | "extend" | "scoped" | "override")
                            )) =>
                    {
                        let [second, third] = call.test_options();
                        let second_object = inspect(ctx, second, &["retry", "timeout"], findings);
                        let third_object = inspect(ctx, third, &["retry", "timeout"], findings);
                        if let Some(arg) = third
                            && second.is_some()
                            && !second_object
                        {
                            let third = super::values::resolve(ctx, arg);
                            if !third_object && !third.is_function() {
                                findings.report(arg.span().start as usize, None);
                            } else if third.is_function()
                                && let Some(e) = second
                                && !super::values::resolve(ctx, e).is_function()
                            {
                                findings.report(e.span().start as usize, None);
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
