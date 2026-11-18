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
fn meta(e: &Expression<'_>) -> bool {
    matches!(e.get_inner_expression(), Expression::ImportMeta(_))
}
fn unbound_name(ctx: &TsCtx<'_>, e: &Expression<'_>, names: &[&str]) -> bool {
    let Expression::Identifier(id) = e.get_inner_expression() else {
        return false;
    };
    names.contains(&id.name.as_str()) && ctx.semantic.is_reference_to_global_variable(id)
}
fn env<'a>(ctx: &TsCtx<'a>, e: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    let m = e.get_member_expr()?;
    (MemberKeys::Resolve(ctx).read(m)? == "env").then(|| m.object())
}
fn process_env(ctx: &TsCtx<'_>, e: &Expression<'_>, suffix: &[&str]) -> bool {
    super::values::path(ctx, e).is_some_and(|p| match super::values::unqualified(&p) {
        ["process" | "node:process", "env", rest @ ..]
        | ["node:process", "default", "env", rest @ ..] => rest == suffix,
        _ => false,
    })
}
fn mode(ctx: &TsCtx<'_>, e: &Expression<'_>) -> Option<Span> {
    let e = e.get_inner_expression();
    if process_env(ctx, e, &["NODE_ENV"]) {
        return Some(e.span());
    }
    let m = e.get_member_expr()?;
    let owner = env(ctx, m.object())?;
    let name = MemberKeys::Resolve(ctx).read(m)?;
    (marker(name, Marker::Mode)
        && (meta(owner) || name == "NODE_ENV" && unbound_name(ctx, owner, &["process"])))
    .then_some(e.span())
}
fn comparison(ctx: &TsCtx<'_>, left: &Expression<'_>, right: &Expression<'_>) -> Option<Span> {
    let test = |e: &Expression<'_>| super::values::literal(ctx, e) == Some("test");
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
fn test_member(ctx: &TsCtx<'_>, object: &Expression<'_>, name: &str) -> bool {
    (marker(name, Marker::Worker) && process_env(ctx, object, &[]))
        || (name == "vitest" && meta(object))
        || (marker(name, Marker::Runner)
            && unbound_name(ctx, object, &["window", "globalThis", "global"]))
}
fn runner_global(ctx: &TsCtx<'_>, expr: &Expression<'_>) -> bool {
    let Expression::Identifier(id) = expr.get_inner_expression() else {
        return false;
    };
    marker(id.name.as_str(), Marker::Runner) && ctx.semantic.is_reference_to_global_variable(id)
}
fn worker(ctx: &TsCtx<'_>, key: &PropertyKey<'_>, computed: bool) -> bool {
    super::values::key(ctx, key).map_or(computed, |name| marker(name, Marker::Worker))
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    if test_file(ctx.path) {
        return;
    }
    for (id, node) in ctx.semantic.nodes().iter_enumerated() {
        let span = match node.kind() {
            kind if let Some(m) = kind.as_member_expression_kind()
                && MemberKeys::Resolve(ctx)
                    .read_kind(m)
                    .is_some_and(|name| test_member(ctx, m.object(), name))
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
