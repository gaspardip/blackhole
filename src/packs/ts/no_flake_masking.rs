use super::test_context::{is_test, root, suite};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{Argument, Expression, ObjectProperty, ObjectPropertyKind, PropertyKey},
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
fn name<'a>(p: &'a ObjectProperty<'a>) -> Option<&'a str> {
    if p.computed {
        return None;
    }
    match &p.key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
        PropertyKey::StringLiteral(s) => s.value.as_str(),
        _ => None,
    }
}
fn inspect(expr: Option<&Expression<'_>>, keys: &[&str], findings: &mut Findings) {
    let Some(Expression::ObjectExpression(o)) = expr else {
        return;
    };
    for p in &o.properties {
        if let ObjectPropertyKind::ObjectProperty(p) = p
            && name(p).is_some_and(|k| k == "retry" || keys.contains(&k))
        {
            findings.report(p.span.start as usize, None);
        }
    }
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for n in ctx.semantic.nodes().iter() {
        match n.kind() {
            AstKind::ObjectProperty(p) if name(p) == Some("test") => {
                inspect(Some(&p.value), &[], findings);
            }
            AstKind::CallExpression(c) if c.callee.is_specific_member_access("vi", "setConfig") => {
                inspect(
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
                inspect(second, &["timeout"], findings);
                if let Some(third) = c.arguments.get(2) {
                    inspect(third.as_expression(), &["timeout"], findings);
                    if !matches!(second, Some(Expression::ObjectExpression(_)))
                        && !matches!(
                            third,
                            Argument::ObjectExpression(_)
                                | Argument::ArrowFunctionExpression(_)
                                | Argument::FunctionExpression(_)
                        )
                    {
                        findings.report(third.span().start as usize, None);
                    }
                }
            }
            _ => (),
        }
    }
}
