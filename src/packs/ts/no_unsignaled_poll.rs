use super::test_context::single_value;
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{AstKind, ast::Expression};
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unsignaled-poll",
    family: Family::Tests,
    provenance: "Port of landmines testing/no-unsignaled-poll.",
    fix: "Await the operation or resolve a promise from the handler that records the event.",
    run: Run::Ts(check),
};
fn recorder(mut e: &Expression<'_>) -> Option<bool> {
    let mut first = true;
    let mut reads = false;
    loop {
        if let Some(m) = e.get_member_expr() {
            let name = m.static_property_name().and_then(oxc::str::JSStr::as_str);
            if name == Some("current") {
                return None;
            }
            reads |= name == Some("mock") || first && matches!(name, Some("length" | "size"));
            e = m.object();
            first = false;
        } else {
            return match e.get_inner_expression() {
                Expression::Identifier(id) if id.name != "current" => {
                    Some(reads || id.name == "mock")
                }
                _ => None,
            };
        }
    }
}
fn assertion(e: &Expression<'_>) -> bool {
    let Expression::CallExpression(c) = e else {
        return false;
    };
    let Some(m) = c.callee.get_member_expr() else {
        return false;
    };
    let object = m.object().get_inner_expression();
    let subject = object
        .get_member_expr()
        .filter(|n| n.static_property_name().is_some_and(|s| s == "not"))
        .map_or(object, |n| n.object().get_inner_expression());
    let Expression::CallExpression(call) = subject else {
        return false;
    };
    if !call.callee.is_specific_id("expect") {
        return false;
    }
    call.arguments
        .first()
        .and_then(|a| a.as_expression())
        .and_then(recorder)
        .is_some_and(|reads| {
            reads
                || matches!(
                    m.static_property_name()
                        .and_then(oxc::str::JSStr::as_str)
                        .unwrap_or(""),
                    "toHaveLength"
                        | "toHaveBeenCalled"
                        | "toHaveBeenCalledOnce"
                        | "toHaveBeenCalledTimes"
                        | "toHaveBeenCalledWith"
                        | "toHaveBeenCalledExactlyOnceWith"
                        | "toHaveBeenLastCalledWith"
                        | "toHaveNthCalledWith"
                        | "toHaveReturned"
                        | "toHaveReturnedTimes"
                        | "toHaveReturnedWith"
                        | "toHaveLastReturnedWith"
                        | "toHaveNthReturnedWith"
                        | "toHaveResolved"
                        | "toHaveResolvedTimes"
                        | "toHaveResolvedWith"
                        | "toHaveLastResolvedWith"
                        | "toHaveNthResolvedWith"
                )
        })
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    for (id, n) in nodes.iter_enumerated() {
        let AstKind::CallExpression(c) = n.kind() else {
            continue;
        };
        let poll = c.callee.is_specific_member_access("expect", "poll");
        if !poll && !c.callee.is_specific_member_access("vi", "waitFor") {
            continue;
        }
        let Some(value) = c
            .arguments
            .first()
            .and_then(|a| a.as_expression())
            .and_then(single_value)
        else {
            continue;
        };
        let reads = recorder(value);
        if reads == Some(true)
            || poll
                && reads.is_some()
                && super::values::outer(ctx, id).is_some_and(|n| match n {
                    AstKind::StaticMemberExpression(m) => m.property.name == "toHaveLength",
                    AstKind::ComputedMemberExpression(m) => m
                        .static_property_name()
                        .is_some_and(|s| s == "toHaveLength"),
                    _ => false,
                })
            || !poll && assertion(value)
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
