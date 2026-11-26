use super::test_context::single_value;
use super::values::{MemberKeys, call_expr, invoked, member_object, receiver, resolve, unwrapped};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::ast::ast::Expression;
use oxc::span::GetSpan;
use std::collections::HashSet;
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unsignaled-poll",
    family: Family::Tests,
    gate: Gate::Blocking,
    provenance: "Port of landmines testing/no-unsignaled-poll.",
    fix: "Await the operation or resolve a promise from the handler that records the event.",
    run: Run::Ts(check),
};
fn recorder<'a>(ctx: &TsCtx<'a>, mut e: &'a Expression<'a>) -> Option<bool> {
    let mut first = true;
    let mut reads = false;
    let mut seen = HashSet::new();
    loop {
        e = unwrapped(ctx, e);
        let resolved = resolve(ctx, e);
        if resolved.get_member_expr().is_some() {
            e = resolved;
        }
        if let Some(m) = e.get_member_expr() {
            let name = MemberKeys(ctx).read(m);
            if name == Some("current") || !seen.insert(m.span()) {
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
fn matcher_subject<'a>(ctx: &TsCtx<'a>, object: &'a Expression<'a>) -> &'a Expression<'a> {
    let object = resolve(ctx, object);
    receiver(ctx, object)
        .filter(|&(_, key)| key == "not")
        .map_or(object, |(object, _)| resolve(ctx, object))
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
    let Some(call) = call_expr(matcher_subject(ctx, object)) else {
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
    let length_subjects: HashSet<_> = nodes
        .iter()
        .filter_map(|n| n.kind().as_member_expression_kind())
        .filter(|&m| MemberKeys(ctx).read_kind(m) == Some("toHaveLength"))
        .filter_map(|m| call_expr(matcher_subject(ctx, member_object(m))).map(|call| call.span))
        .collect();
    for n in nodes.iter() {
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
            || poll && reads.is_some() && length_subjects.contains(&c.span)
            || !poll && assertion(ctx, value)
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
