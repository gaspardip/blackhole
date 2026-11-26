use super::{MemberKeys, Origin, origin_path, resolve, unwrapped};
use crate::engine::{Findings, TsCtx};
use crate::packs::ts::{no_flake_masking, test_context::Runner};
use oxc::ast::{AstKind, ast::Expression};
use oxc::span::GetSpan;
use std::collections::HashSet;

fn normalized_prefix<'a>(ctx: &TsCtx<'a>, callee: &'a Expression<'a>) -> usize {
    let callee = resolve(ctx, callee);
    if let Some((Origin::Global, path)) = origin_path(ctx, callee) {
        match path.as_slice() {
            ["Reflect", "apply"] => return 3,
            ["Function", "prototype", "call", "call"] => return 2,
            _ => {}
        }
    }
    let Some(member) = callee.get_member_expr() else {
        return 0;
    };
    if origin_path(ctx, member.object()).is_none()
        && !super::super::strict_msw::tracked(ctx, member.object())
    {
        return 0;
    }
    match MemberKeys(ctx).read(member) {
        Some("bind" | "call") => 1,
        Some("apply") => 2,
        _ => 0,
    }
}

fn mock_value(origin: Origin, path: &[&str]) -> bool {
    let ((Origin::Global, names) | (_, ["vitest" | "@jest/globals", names @ ..])) = (origin, path)
    else {
        return false;
    };
    matches!(names, ["vi" | "vitest" | "jest", method] if super::super::double_mutations::banned_mock_method(method))
        || matches!(
            names,
            ["vi" | "vitest" | "jest"]
                | [
                    "vi" | "vitest" | "jest",
                    "mock" | "doMock" | "mockObject" | "fn"
                ]
        )
}

pub(super) struct Escape {
    rule: &'static str,
    start: usize,
    message: String,
}

fn owner<'a>(ctx: &TsCtx<'a>, value: &'a Expression<'a>) -> Option<&'static str> {
    if super::super::strict_msw::tracked(ctx, value) {
        return Some("ts/strict-msw");
    }
    if super::receiver(ctx, value).is_some_and(|(owner, key)| {
        super::super::double_mutations::replacement_method(ctx, owner, key)
    }) {
        return Some("ts/no-test-doubles");
    }
    if let Expression::ThisExpression(t) = super::resolve(ctx, value)
        && super::super::test_context::enclosing_runners(ctx, t.node_id.get())
            .next()
            .is_some_and(|r| r.test_or_suite())
    {
        return Some("ts/no-skipped-tests");
    }
    let (origin, path) = origin_path(ctx, value)?;
    if mock_value(origin, &path) {
        return Some("ts/no-test-doubles");
    }
    if matches!(
        (origin, path.as_slice()),
        (
            Origin::Module | Origin::Required,
            ["node:timers/promises" | "timers/promises", "setTimeout"]
        )
    ) {
        return Some("ts/no-fixed-sleep");
    }
    let runner = Runner::from_path(origin, &path)?;
    let names = runner.short();
    let single_name = matches!(names, Some([_]));
    if (origin != Origin::Global && runner.is_empty())
        || (runner.test_or_suite() && (runner.skipped || single_name))
        || (single_name && runner.legacy_skip())
    {
        return Some("ts/no-skipped-tests");
    }
    if no_flake_masking::api(&runner).is_some() {
        return Some(no_flake_masking::RULE.id);
    }
    match names? {
        ["Bun", "sleep"] | ["scheduler", "wait"] | ["setTimeout"] => Some("ts/no-fixed-sleep"),
        _ => None,
    }
}

fn inspect_value<'a>(ctx: &TsCtx<'a>, value: &'a Expression<'a>, escapes: &mut Vec<Escape>) {
    let mut pending = vec![value];
    let mut seen = HashSet::new();
    while let Some(value) = pending.pop() {
        let value = unwrapped(ctx, value);
        let resolved = resolve(ctx, value);
        if let Expression::ConditionalExpression(branch) = resolved {
            if seen.insert(branch.span) {
                pending.extend([&branch.alternate, &branch.consequent]);
            }
            continue;
        }
        let mut rules = Vec::new();
        if let Some(rule) = owner(ctx, value) {
            rules.push(rule);
        } else if let Expression::Identifier(_) = resolved {
            super::any_value(ctx, value, |candidate| {
                if let Some(rule) = owner(ctx, candidate)
                    && !rules.contains(&rule)
                {
                    rules.push(rule);
                }
                false
            });
        }
        for rule in rules {
            let span = value
                .get_member_expr()
                .map_or_else(|| value.span(), |m| super::member_span(ctx, m));
            let name = span.source_text(ctx.text);
            escapes.push(Escape {
                rule,
                start: span.start as usize,
                message: format!("cannot read how `{name}` is used here; call it by name"),
            });
        }
    }
}

fn escapes(ctx: &TsCtx<'_>) -> Vec<Escape> {
    let nodes = ctx.semantic.nodes();
    let mut targets = HashSet::new();
    for node in nodes.iter() {
        if let Some(site) = super::site(ctx, node.kind()) {
            targets.extend(site.call.intrinsic_targets.iter().copied());
        }
    }
    let mut escapes = Vec::new();
    let mut inspect = |value| inspect_value(ctx, value, &mut escapes);
    let mut fields = Vec::new();
    let mut projections = HashSet::new();
    for node in nodes.iter() {
        if let Some(m) = node.kind().as_member_expression_kind()
            && let Some(prop) = MemberKeys(ctx)
                .read_kind(m)
                .and_then(|k| super::projection(ctx, super::member_object(m), k))
        {
            projections.insert(prop.span);
        }
        match node.kind() {
            AstKind::CallExpression(call) => {
                for arg in call
                    .arguments
                    .iter()
                    .skip(normalized_prefix(ctx, &call.callee))
                    .filter_map(|a| a.as_expression())
                {
                    if !targets.contains(&arg.span()) {
                        inspect(arg);
                    }
                }
            }
            AstKind::ArrayExpression(array) => {
                for element in array.elements.iter().filter_map(|el| el.as_expression()) {
                    inspect(element);
                }
            }
            AstKind::NewExpression(new) => {
                for arg in new.arguments.iter().filter_map(|a| a.as_expression()) {
                    inspect(arg);
                }
            }
            AstKind::ReturnStatement(ret) => {
                if let Some(arg) = &ret.argument {
                    inspect(arg);
                }
            }
            AstKind::ArrowFunctionExpression(f) => {
                if let Some(body) = f.body.as_expression() {
                    inspect(body);
                }
            }
            AstKind::PropertyDefinition(p) => {
                if let Some(v) = &p.value {
                    inspect(v);
                }
            }
            AstKind::FormalParameter(p) => {
                if let Some(v) = &p.initializer {
                    inspect(v);
                }
            }
            AstKind::AssignmentExpression(a) if a.left.as_member_expression().is_some() => {
                inspect(&a.right);
            }
            AstKind::ObjectProperty(p) => fields.push(p),
            AstKind::SpreadElement(s) => inspect(&s.argument),
            AstKind::ComputedMemberExpression(m)
                if super::literal(ctx, &m.expression).is_none() =>
            {
                inspect(&m.object);
            }
            _ => {}
        }
    }
    for prop in fields {
        if !projections.contains(&prop.span) {
            inspect(&prop.value);
        }
    }
    escapes
}

pub(in crate::packs::ts) fn report(ctx: &TsCtx<'_>, rule: &str, findings: &mut Findings) {
    for usage in ctx.resolution.escapes.get_or_init(|| escapes(ctx)) {
        if usage.rule == rule {
            findings.report(usage.start, Some(&usage.message));
        }
    }
}
