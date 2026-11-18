use super::test_context::{is_test, member, root, single};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{Expression, ImportDeclarationSpecifier, NewExpression},
};
use std::collections::HashSet;
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-fixed-sleep",
    family: Family::Tests,
    provenance: "Port of landmines testing/no-fixed-sleep.",
    fix: "Await the operation or a promise resolved by its handler. Use fake timers to test elapsed time.",
    run: Run::Ts(check),
};
fn promise(n: &NewExpression<'_>) -> bool {
    if !matches!(&n.callee,Expression::Identifier(id) if id.name=="Promise") {
        return false;
    }
    let Some(callback) = n.arguments.first().and_then(|a| a.as_expression()) else {
        return false;
    };
    let params = match callback {
        Expression::ArrowFunctionExpression(f) => &f.params,
        Expression::FunctionExpression(f) => &f.params,
        _ => return false,
    };
    let Some(resolve) = params
        .items
        .first()
        .and_then(|p| p.pattern.get_binding_identifier())
    else {
        return false;
    };
    let Some(Expression::CallExpression(c)) = single(callback, false) else {
        return false;
    };
    if !(matches!(&c.callee,Expression::Identifier(id) if id.name=="setTimeout")
        || member(&c.callee, "window", "setTimeout")
        || member(&c.callee, "globalThis", "setTimeout"))
    {
        return false;
    }
    match c.arguments.first().and_then(|a| a.as_expression()) {
        Some(Expression::Identifier(id)) => id.name == resolve.name,
        Some(Expression::ArrowFunctionExpression(f)) => {
            matches!(f.body.as_expression(),Some(Expression::CallExpression(c)) if matches!(&c.callee,Expression::Identifier(id) if id.name==resolve.name))
        }
        _ => false,
    }
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    let mut timers = HashSet::new();
    for n in nodes.iter() {
        match n.kind() {
            AstKind::CallExpression(c) if member(&c.callee, "vi", "useFakeTimers") => return,
            AstKind::ImportDeclaration(i)
                if matches!(
                    i.source.value.as_str(),
                    Some("node:timers/promises" | "timers/promises")
                ) =>
            {
                for spec in i.specifiers.iter().flatten() {
                    if let ImportDeclarationSpecifier::ImportSpecifier(s) = spec
                        && s.imported.name() == "setTimeout"
                    {
                        timers.insert(s.local.name.as_str());
                    }
                }
            }
            _ => (),
        }
    }
    for (id, n) in nodes.iter_enumerated() {
        let AstKind::AwaitExpression(a) = n.kind() else {
            continue;
        };
        let sleep = match &a.argument {
            Expression::NewExpression(n) => promise(n),
            Expression::CallExpression(c) => {
                member(&c.callee, "scheduler", "wait")
                    || member(&c.callee, "Bun", "sleep")
                    || matches!(&c.callee,Expression::Identifier(i) if timers.contains(i.name.as_str()))
            }
            _ => false,
        };
        if sleep
            && let Some(f) = nodes.ancestors(id).find(|n| {
                matches!(
                    n.kind(),
                    AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
                )
            })
            && let AstKind::CallExpression(c) = nodes.parent_kind(f.id())
            && root(&c.callee).is_some_and(|s| {
                is_test(s) || matches!(s, "beforeEach" | "beforeAll" | "afterEach" | "afterAll")
            })
        {
            findings.report(a.span.start as usize, None);
        }
    }
}
