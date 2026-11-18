use super::test_context::single_value;
use super::values::MemberKeys;
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
            let name = MemberKeys::Literal.read(m);
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
fn assertion<'a>(ctx: &TsCtx<'a>, e: &'a Expression<'a>) -> bool {
    let Some(c) = super::values::call_expr(e) else {
        return false;
    };
    let Some(m) = c.callee.get_member_expr() else {
        return false;
    };
    let object = m.object().get_inner_expression();
    let subject = object
        .get_member_expr()
        .filter(|n| MemberKeys::Literal.read(n) == Some("not"))
        .map_or(object, |n| n.object().get_inner_expression());
    let Expression::CallExpression(call) = subject else {
        return false;
    };
    if super::test_context::runner(ctx, &super::values::invoked(ctx, call)) != ["expect"] {
        return false;
    }
    call.arguments
        .first()
        .and_then(|a| a.as_expression())
        .and_then(recorder)
        .is_some_and(|reads| {
            reads
                || matches!(
                    MemberKeys::Literal.read(m).unwrap_or(""),
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
        let call = super::values::invoked(ctx, c);
        let path = super::test_context::runner(ctx, &call);
        let poll = path == ["expect", "poll"];
        if !poll && path != ["vi", "waitFor"] {
            continue;
        }
        let Some(value) = call
            .arg(0)
            .map(|e| super::values::resolve(ctx, e))
            .and_then(single_value)
        else {
            continue;
        };
        let reads = recorder(value);
        if reads == Some(true)
            || poll
                && reads.is_some()
                && super::values::outer(ctx, id)
                    .and_then(|n| n.as_member_expression_kind())
                    .and_then(|m| MemberKeys::Literal.read_kind(m))
                    == Some("toHaveLength")
            || !poll && assertion(ctx, value)
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
