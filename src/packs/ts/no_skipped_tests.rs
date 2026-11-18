use super::test_context::{chain_property, is_test, suite, test_file};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{AstKind, ast::Expression},
    span::GetSpan,
};
use std::collections::BTreeSet;
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-skipped-tests",
    family: Family::Tests,
    provenance: "Agents skip, focus or stub out a failing test to get a green run.",
    fix: "Make the test pass or delete it with the behaviour it covered. A skipped test proves nothing.",
    run: Run::Ts(check),
};
pub(super) fn modifier(name: &str) -> bool {
    matches!(
        name,
        "skip" | "only" | "todo" | "skipIf" | "runIf" | "fixme" | "fails" | "failing"
    )
}
fn body_skip(e: &Expression<'_>) -> Option<usize> {
    if let Some(member) = e.get_member_expr()
        && matches!(
            member.object().get_inner_expression(),
            Expression::ThisExpression(_)
        )
        && let Some((span, name)) = member.static_property_info()
        && name == "skip"
    {
        return Some(span.start as usize);
    }
    match e.get_inner_expression() {
        Expression::Identifier(i) if i.name == "pending" => Some(i.span.start as usize),
        _ => None,
    }
}
fn skip_option<'a>(ctx: &TsCtx<'a>, arg: &'a Expression<'a>) -> Option<usize> {
    let options = super::values::properties(ctx, arg)?;
    options.named.into_iter().find_map(|(key,property)|{
        let enabled=matches!(super::values::resolve(ctx,&property.value),Expression::BooleanLiteral(value) if value.value);
        (matches!(key,"skip"|"todo") && enabled).then_some(property.key.span().start as usize)
    })
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut locations = BTreeSet::new();
    let nodes = ctx.semantic.nodes();
    let is_test_file = test_file(ctx.path);
    for (id, n) in nodes.iter_enumerated() {
        let AstKind::CallExpression(c) = n.kind() else {
            continue;
        };
        let call = super::values::invoked(ctx, c);
        let path = super::test_context::runner(ctx, &call);
        let base = path.first().copied().unwrap_or("");
        let position = if is_test(base) || suite(base) {
            if path.skipped {
                Some(
                    chain_property(ctx, call.callee, modifier)
                        .unwrap_or(call.callee.span().start as usize),
                )
            } else {
                call.arg(1).and_then(|arg| skip_option(ctx, arg))
            }
        } else if matches!(base, "xit" | "xtest" | "xdescribe" | "fit" | "fdescribe") {
            Some(call.callee.span().start as usize)
        } else if is_test_file
            && let Some(at) = body_skip(call.callee)
            && super::values::enclosing_functions(ctx, id).any(|f| {
                let Some(AstKind::CallExpression(parent)) = super::values::outer(ctx, f.id())
                else {
                    return false;
                };
                super::test_context::runner(ctx, &super::values::invoked(ctx, parent))
                    .first()
                    .is_some_and(|s| is_test(s))
            })
        {
            Some(at)
        } else {
            None
        };
        locations.extend(position);
    }
    for at in locations {
        findings.report(at, None);
    }
}
