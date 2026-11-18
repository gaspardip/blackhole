use super::test_context::{direct_member, single_value};
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
        if let Expression::ChainExpression(c) = e
            && let Some(m) = c.expression.as_member_expression()
        {
            let name = m.static_property_name();
            if name.is_some_and(|s| s == "current") {
                return None;
            }
            reads |= name.is_some_and(|s| s == "mock" || first && (s == "length" || s == "size"));
            e = m.object();
            first = false;
            continue;
        }
        if let Some(m) = e.as_member_expression() {
            if let Expression::StaticMemberExpression(p) = e {
                let name = p.property.name.as_str();
                if name == "current" {
                    return None;
                }
                reads |= name == "mock" || first && matches!(name, "length" | "size");
            }
            e = m.object();
            first = false;
        } else {
            return match e {
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
    let Expression::StaticMemberExpression(m) = &c.callee else {
        return false;
    };
    let subject = match &m.object {
        Expression::StaticMemberExpression(n) if n.property.name == "not" => &n.object,
        other => other,
    };
    let Expression::CallExpression(call) = subject else {
        return false;
    };
    if !matches!(&call.callee,Expression::Identifier(id) if id.name=="expect") {
        return false;
    }
    call.arguments
        .first()
        .and_then(|a| a.as_expression())
        .and_then(recorder)
        .is_some_and(|reads| {
            reads
                || matches!(
                    m.property.name.as_str(),
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
        let poll = direct_member(&c.callee, "expect", "poll");
        if !poll && !direct_member(&c.callee, "vi", "waitFor") {
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
                && matches!(nodes.parent_kind(id),AstKind::StaticMemberExpression(m) if m.property.name=="toHaveLength")
            || !poll && assertion(value)
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
