use super::test_context::{is_test, root, suite};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{Argument, Expression, ObjectPropertyKind},
    },
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
    let Some(Expression::ObjectExpression(o)) = expr.map(Expression::get_inner_expression) else {
        return;
    };
    let mut unreadable = false;
    for p in &o.properties {
        if let ObjectPropertyKind::ObjectProperty(p) = p {
            match super::values::key(ctx, &p.key, p.computed) {
                Some(k) if k == "retry" || keys.contains(&k) => {
                    findings.report(p.span.start as usize, None);
                }
                None if p.computed => unreadable = true,
                _ => (),
            }
        }
    }
    if unreadable {
        findings.report(o.span.start as usize, None);
    }
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for n in ctx.semantic.nodes().iter() {
        match n.kind() {
            AstKind::ObjectProperty(p)
                if super::values::key(ctx, &p.key, p.computed) == Some("test") =>
            {
                inspect(ctx, Some(&p.value), &[], findings);
            }
            AstKind::CallExpression(c) if c.callee.is_specific_member_access("vi", "setConfig") => {
                inspect(
                    ctx,
                    c.arguments.first().and_then(Argument::as_expression),
                    &["testTimeout", "hookTimeout"],
                    findings,
                );
            }
            AstKind::CallExpression(c)
                if root(&c.callee).is_some_and(|s| is_test(s) || suite(s))
                    && !super::test_context::static_method(
                        &c.callee,
                        &["each", "for", "extend", "scoped", "override"],
                    ) =>
            {
                let second = c.arguments.get(1).and_then(Argument::as_expression);
                inspect(ctx, second, &["timeout"], findings);
                if let Some(third) = c.arguments.get(2) {
                    inspect(ctx, third.as_expression(), &["timeout"], findings);
                    if !matches!(
                        second.map(Expression::get_inner_expression),
                        Some(Expression::ObjectExpression(_))
                    ) && !matches!(
                        third.as_expression().map(Expression::get_inner_expression),
                        Some(
                            Expression::ObjectExpression(_)
                                | Expression::ArrowFunctionExpression(_)
                                | Expression::FunctionExpression(_)
                        )
                    ) {
                        findings.report(third.span().start as usize, None);
                    }
                }
            }
            _ => (),
        }
    }
}
