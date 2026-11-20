use super::test_context::single_value;
use super::values::{MemberKeys, call_expr, invoked, receiver, resolve, unwrapped};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::ast::Expression;
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unsignaled-poll",
    family: Family::Tests,
    provenance: "Port of landmines testing/no-unsignaled-poll.",
    fix: "Await the operation or resolve a promise from the handler that records the event.",
    run: Run::Ts(check),
};
fn recorder<'a>(ctx: &TsCtx<'a>, mut e: &'a Expression<'a>) -> Option<bool> {
    e = unwrapped(e);
    let resolved = resolve(ctx, e);
    if resolved.get_member_expr().is_some() {
        e = resolved;
    }
    let mut first = true;
    let mut reads = false;
    loop {
        if let Some(m) = e.get_member_expr() {
            let name = MemberKeys(ctx).read(m);
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
    let Some(call) = call_expr(resolve(ctx, e)) else {
        return false;
    };
    let matcher = invoked(ctx, call);
    let (object, name) = if let Some(member) = matcher.callee.get_member_expr() {
        (member.object(), MemberKeys(ctx).read(member))
    } else if let Some((object, name)) = receiver(ctx, matcher.callee) {
        (object, Some(name))
    } else {
        return false;
    };
    let object = resolve(ctx, object);
    let subject = receiver(ctx, object)
        .filter(|&(_, key)| key == "not")
        .map_or(object, |(object, _)| resolve(ctx, object));
    let Some(call) = call_expr(subject) else {
        return false;
    };
    let expect = invoked(ctx, call);
    if super::test_context::runner(ctx, &expect) != ["expect"] {
        return false;
    }
    expect
        .arg(0)
        .and_then(|e| recorder(ctx, e))
        .is_some_and(|reads| {
            reads
                || matches!(
                    name.unwrap_or_default(),
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
        let Some(c) = super::values::site(ctx, n.kind()) else {
            continue;
        };
        let call = c.call;
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
        let reads = recorder(ctx, value);
        if reads == Some(true)
            || poll
                && reads.is_some()
                && super::values::outer(ctx, id)
                    .and_then(|n| n.as_member_expression_kind())
                    .and_then(|m| MemberKeys(ctx).read_kind(m))
                    == Some("toHaveLength")
            || !poll && assertion(ctx, value)
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
