use super::test_context::{chain_property, is_test, root, suite, test_file};
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
fn modifier(name: &str) -> bool {
    matches!(
        name,
        "skip" | "only" | "todo" | "skipIf" | "runIf" | "fixme" | "fails" | "failing"
    )
}
fn focused_or_skipped(e: &Expression<'_>) -> bool {
    let Some(id) = e.get_identifier_reference() else {
        return false;
    };
    matches!(
        id.name.as_str(),
        "xit" | "xtest" | "xdescribe" | "fit" | "fdescribe"
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
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut locations = BTreeSet::new();
    let nodes = ctx.semantic.nodes();
    let is_test_file = test_file(ctx.path);
    for (id, n) in nodes.iter_enumerated() {
        let AstKind::CallExpression(c) = n.kind() else {
            continue;
        };
        let position = if root(&c.callee).is_some_and(|s| is_test(s) || suite(s)) {
            chain_property(&c.callee, modifier)
        } else if focused_or_skipped(&c.callee) {
            Some(c.callee.span().start as usize)
        } else if is_test_file
            && let Some(at) = body_skip(&c.callee)
            && super::values::enclosing_functions(ctx, id).any(|f| {
                match nodes.parent_kind(f.id()) {
                    AstKind::CallExpression(parent) => root(&parent.callee).is_some_and(is_test),
                    _ => false,
                }
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
