use super::test_context::{is_test, runner, suite};
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
fn inspect(ctx: &TsCtx<'_>, expr: Option<&Expression<'_>>, keys: &[&str], findings: &mut Findings) {
    let Some(expr) = expr else {
        return;
    };
    let Some(opts) = super::values::properties(ctx, expr) else {
        return;
    };
    for (k, p) in opts.named {
        if keys.contains(&k) {
            findings.report(p.span.start as usize, None);
        }
    }
    if opts.unknown {
        findings.report(expr.get_inner_expression().span().start as usize, None);
    }
}
fn config(ctx: &TsCtx<'_>, arg: Option<&Expression<'_>>, keys: &[&str], findings: &mut Findings) {
    if let Some(arg) = arg {
        if matches!(
            super::values::resolve(ctx, arg),
            Expression::ObjectExpression(_)
        ) {
            inspect(ctx, Some(arg), keys, findings);
        } else {
            findings.report(arg.span().start as usize, None);
        }
    }
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for n in ctx.semantic.nodes().iter() {
        match n.kind() {
            AstKind::ObjectProperty(p) if super::values::key(ctx, &p.key) == Some("test") => {
                inspect(ctx, Some(&p.value), &["retry"], findings);
            }
            AstKind::CallExpression(c) => {
                let call = super::values::invoked(ctx, c);
                let p = runner(ctx, &call);
                match p.as_slice() {
                    ["vi", "setConfig"] => config(
                        ctx,
                        call.arg(0),
                        &["retry", "testTimeout", "hookTimeout"],
                        findings,
                    ),
                    ["test", "describe", "configure"] => {
                        config(ctx, call.arg(0), &["retries"], findings)
                    }
                    ["test", "slow"] => findings.report(c.span.start as usize, None),
                    [base, ..]
                        if (is_test(base) || suite(base))
                            && (super::values::call_expr(call.callee).is_some()
                                || !matches!(
                                    p.last(),
                                    Some(&("each" | "for" | "extend" | "scoped" | "override"))
                                )) =>
                    {
                        let second = call.arg(1);
                        let third = call.arg(2);
                        inspect(ctx, second, &["retry", "timeout"], findings);
                        inspect(ctx, third, &["retry", "timeout"], findings);
                        if let Some(arg) = third
                            && second.is_some_and(|e| super::values::properties(ctx, e).is_none())
                        {
                            let third = super::values::resolve(ctx, arg);
                            if !matches!(
                                third,
                                Expression::ObjectExpression(_)
                                    | Expression::ArrowFunctionExpression(_)
                                    | Expression::FunctionExpression(_)
                            ) {
                                findings.report(arg.span().start as usize, None);
                            } else if matches!(
                                third,
                                Expression::ArrowFunctionExpression(_)
                                    | Expression::FunctionExpression(_)
                            ) && let Some(e) = second
                                && !matches!(
                                    super::values::resolve(ctx, e),
                                    Expression::ArrowFunctionExpression(_)
                                        | Expression::FunctionExpression(_)
                                )
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
