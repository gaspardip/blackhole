use super::test_context::test_file;
use super::values::MemberKeys;
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{
        AstKind,
        ast::{
            AssignmentTarget, AssignmentTargetProperty, BindingPattern, Expression, PropertyKey,
        },
    },
    span::{GetSpan, Span},
    syntax::node::NodeId,
};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-test-aware-code",
    family: Family::Tests,
    provenance: "ImpossibleBench and similar reports observed agents special-casing a test run in production code.",
    fix: "Production code must not know it is under test. Inject the dependency or the configuration instead.",
    run: Run::Ts(check),
};
#[derive(Clone, Copy, PartialEq)]
enum Marker {
    Worker,
    Runner,
    Mode,
}
const NAMES: &[(&str, Marker)] = &[
    ("JEST_WORKER_ID", Marker::Worker),
    ("VITEST", Marker::Worker),
    ("VITEST_WORKER_ID", Marker::Worker),
    ("VITEST_POOL_ID", Marker::Worker),
    ("PLAYWRIGHT_TEST", Marker::Worker),
    ("CYPRESS", Marker::Worker),
    ("jest", Marker::Runner),
    ("vi", Marker::Runner),
    ("vitest", Marker::Runner),
    ("describe", Marker::Runner),
    ("it", Marker::Runner),
    ("expect", Marker::Runner),
    ("Cypress", Marker::Runner),
    ("NODE_ENV", Marker::Mode),
    ("MODE", Marker::Mode),
];
fn marker(key: &str, kind: Marker) -> bool {
    NAMES.contains(&(key, kind))
}
fn process_env<'a>(ctx: &TsCtx<'a>, e: &'a Expression<'a>, suffix: &[&str]) -> bool {
    super::values::path(ctx, e).is_some_and(|p| match p.as_slice() {
        ["process" | "node:process", "env", rest @ ..]
        | ["node:process", "default", "env", rest @ ..] => rest == suffix,
        _ => false,
    })
}
fn mode<'a>(ctx: &TsCtx<'a>, e: &'a Expression<'a>) -> Option<Span> {
    let e = e.get_inner_expression();
    if process_env(ctx, e, &["NODE_ENV"]) {
        return Some(e.span());
    }
    super::values::path(ctx, e)
        .is_some_and(
            |p| matches!(p.as_slice(),["import.meta","env",name] if marker(name,Marker::Mode)),
        )
        .then_some(e.span())
}
fn comparison<'a>(
    ctx: &TsCtx<'a>,
    left: &'a Expression<'a>,
    right: &'a Expression<'a>,
) -> Option<Span> {
    let test = |e: &'a Expression<'a>| super::values::literal(ctx, e) == Some("test");
    if test(right) {
        mode(ctx, left)
    } else if test(left) {
        mode(ctx, right)
    } else {
        None
    }
}
fn read(ctx: &TsCtx<'_>, id: NodeId, span: Span) -> bool {
    match super::values::outer(ctx, id) {
        Some(AstKind::AssignmentExpression(a)) => !a.operator.is_assign() || a.left.span() != span,
        Some(AstKind::AssignmentTargetPropertyProperty(p)) => p.binding.span() != span,
        Some(AstKind::AssignmentTargetWithDefault(p)) => p.binding.span() != span,
        Some(AstKind::UnaryExpression(u)) => !u.operator.is_delete(),
        _ => true,
    }
}
fn test_member<'a>(ctx: &TsCtx<'a>, object: &'a Expression<'a>, name: &str) -> bool {
    (marker(name, Marker::Worker) && process_env(ctx, object, &[]))
        || (name == "vitest"
            && super::values::path(ctx, object).is_some_and(|p| p == ["import.meta"]))
        || (marker(name, Marker::Runner)
            && super::values::path(ctx, object).is_some_and(|p| p.is_empty()))
}
fn runner_global<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> bool {
    let Expression::Identifier(id) = expr.get_inner_expression() else {
        return false;
    };
    marker(id.name.as_str(), Marker::Runner) && ctx.semantic.is_reference_to_global_variable(id)
}
fn worker<'a>(ctx: &TsCtx<'a>, key: &'a PropertyKey<'a>, computed: bool) -> bool {
    super::values::key(ctx, key).map_or(computed, |name| marker(name, Marker::Worker))
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    if test_file(ctx.path) {
        return;
    }
    for (id, node) in ctx.semantic.nodes().iter_enumerated() {
        let span = match node.kind() {
            kind if let Some(m) = kind.as_member_expression_kind()
                && MemberKeys(ctx).read_kind(m).is_some_and(|name| {
                    test_member(ctx, super::values::member_object(m), name)
                })
                && read(ctx, id, node.span()) =>
            {
                Some(node.span())
            }
            AstKind::UnaryExpression(u)
                if u.operator.is_typeof() && runner_global(ctx, &u.argument) =>
            {
                Some(u.span)
            }
            AstKind::BinaryExpression(b) if b.operator.is_equality() => {
                comparison(ctx, &b.left, &b.right)
            }
            AstKind::SwitchStatement(s) => {
                let mut found = std::collections::BTreeSet::new();
                for c in &s.cases {
                    if let Some(at) = c
                        .test
                        .as_ref()
                        .and_then(|e| comparison(ctx, &s.discriminant, e))
                    {
                        found.insert(at.start);
                    }
                }
                for at in found {
                    findings.report(at as usize, None);
                }
                None
            }
            AstKind::VariableDeclarator(v)
                if v.init.as_ref().is_some_and(|e| process_env(ctx, e, &[])) =>
            {
                if let BindingPattern::ObjectPattern(p) = &v.id {
                    for prop in &p.properties {
                        if worker(ctx, &prop.key, prop.computed) {
                            findings.report(prop.key.span().start as usize, None);
                        }
                    }
                }
                None
            }
            AstKind::AssignmentExpression(a) if process_env(ctx, &a.right, &[]) => {
                if let AssignmentTarget::ObjectAssignmentTarget(o) = &a.left {
                    for p in &o.properties {
                        match p {
                            AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(p)
                                if marker(p.binding.name.as_str(), Marker::Worker) =>
                            {
                                findings.report(p.span.start as usize, None);
                            }
                            AssignmentTargetProperty::AssignmentTargetPropertyProperty(p)
                                if worker(ctx, &p.name, p.computed) =>
                            {
                                findings.report(p.name.span().start as usize, None);
                            }
                            _ => (),
                        }
                    }
                }
                None
            }
            _ => None,
        };
        if let Some(span) = span {
            findings.report(span.start as usize, None);
        }
    }
}
