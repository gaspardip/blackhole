use super::test_context::test_file;
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
fn env<'a>(e: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    let m = e.get_member_expr()?;
    (m.static_property_name()? == "env").then(|| m.object())
}
fn mode(ctx: &TsCtx<'_>, e: &Expression<'_>) -> Option<Span> {
    let e = e.get_inner_expression();
    let m = e.get_member_expr()?;
    let owner = env(m.object())?;
    let name = m.static_property_name()?.as_str()?;
    (marker(name, Marker::Mode)
        && (meta(owner) || name == "NODE_ENV" && unbound_name(ctx, owner, &["process"])))
    .then_some(e.span())
}
fn comparison(ctx: &TsCtx<'_>, left: &Expression<'_>, right: &Expression<'_>) -> Option<Span> {
    let test = |e: &Expression<'_>| matches!(e.get_inner_expression(),Expression::StringLiteral(s) if s.value=="test");
    if test(right) {
        mode(ctx, left)
    } else if test(left) {
        mode(ctx, right)
    } else {
        None
    }
}
fn read(ctx: &TsCtx<'_>, id: NodeId, span: Span) -> bool {
    let parent = ctx.semantic.nodes().ancestors(id).find(|n| {
        !matches!(
            n.kind(),
            AstKind::ParenthesizedExpression(_) | AstKind::ChainExpression(_)
        )
    });
    match parent.map(oxc::semantic::AstNode::kind) {
        Some(AstKind::AssignmentExpression(a)) => !a.operator.is_assign() || a.left.span() != span,
        Some(AstKind::UnaryExpression(u)) => !u.operator.is_delete(),
        _ => true,
    }
}
fn test_member(ctx: &TsCtx<'_>, object: &Expression<'_>, name: &str) -> bool {
    (marker(name, Marker::Worker)
        && env(object).is_some_and(|e| unbound_name(ctx, e, &["process"])))
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
    super::values::key(ctx, key, computed).map_or(computed, |name| marker(name, Marker::Worker))
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    if test_file(ctx.path) {
        return;
    }
    for (id, node) in ctx.semantic.nodes().iter_enumerated() {
        let span = match node.kind() {
            AstKind::StaticMemberExpression(m)
                if test_member(ctx, &m.object, m.property.name.as_str())
                    && read(ctx, id, m.span) =>
            {
                Some(m.span)
            }
            AstKind::ComputedMemberExpression(m)
                if m.static_property_name()
                    .and_then(oxc::str::JSStr::as_str)
                    .is_some_and(|key| test_member(ctx, &m.object, key))
                    && read(ctx, id, m.span) =>
            {
                Some(m.span)
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
                if v.init
                    .as_ref()
                    .and_then(env)
                    .is_some_and(|e| unbound_name(ctx, e, &["process"])) =>
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
            AstKind::AssignmentExpression(a)
                if env(&a.right).is_some_and(|e| unbound_name(ctx, e, &["process"])) =>
            {
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
