use super::values::symbol;
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{AssignmentTarget, Expression, ObjectPropertyKind},
};
use oxc::syntax::symbol::SymbolId;
use std::collections::HashMap;

pub(crate) const RULE: Rule = Rule {
    id: "ts/strict-msw",
    family: Family::Tests,
    provenance: "Port of landmines testing/strict-msw.",
    fix: "Start MSW with onUnhandledRequest set to error. Add handlers for the requests the test makes.",
    run: Run::Ts(check),
};
#[derive(Clone, Copy, PartialEq)]
enum Factory {
    Server,
    Worker,
    Ambiguous,
}
fn factory(ctx: &TsCtx<'_>, e: &Expression<'_>) -> Option<Factory> {
    let Expression::CallExpression(c) = e else {
        return None;
    };
    let Expression::Identifier(id) = &c.callee else {
        return None;
    };
    let (source, super::values::Imported::Named(name)) = super::values::import_of(ctx, id)? else {
        return None;
    };
    if source != "msw" && !source.starts_with("msw/") {
        return None;
    }
    match name {
        "setupServer" => Some(Factory::Server),
        "setupWorker" => Some(Factory::Worker),
        _ => None,
    }
}
fn strict(ctx: &TsCtx<'_>, opts: Option<&Expression<'_>>) -> bool {
    let Some(Expression::ObjectExpression(o)) = opts else {
        return false;
    };
    for prop in o.properties.iter().rev() {
        let ObjectPropertyKind::ObjectProperty(p) = prop else {
            return false;
        };
        match super::values::key(ctx, &p.key, p.computed) {
            Some("onUnhandledRequest") => {
                return match &p.value {
                    Expression::StringLiteral(s) => s.value == "error",
                    Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => {
                        true
                    }
                    _ => false,
                };
            }
            None => return false,
            _ => (),
        }
    }
    false
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut factories = HashMap::<SymbolId, Factory>::new();
    for node in ctx.semantic.nodes().iter() {
        let (symbol, value) = match node.kind() {
            AstKind::VariableDeclarator(v) => (
                v.id.get_binding_identifier()
                    .and_then(|b| b.symbol_id.get()),
                v.init.as_ref(),
            ),
            AstKind::AssignmentExpression(a) => {
                if let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                    (symbol(ctx, id), Some(&a.right))
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        if let (Some(symbol), Some(kind)) = (symbol, value.and_then(|v| factory(ctx, v))) {
            factories
                .entry(symbol)
                .and_modify(|previous| {
                    if *previous != kind {
                        *previous = Factory::Ambiguous;
                    }
                })
                .or_insert(kind);
        }
    }
    for node in ctx.semantic.nodes().iter() {
        let AstKind::CallExpression(c) = node.kind() else {
            continue;
        };
        let Some(m) = c.callee.get_member_expr() else {
            continue;
        };
        let kind = match m.object().get_inner_expression() {
            Expression::Identifier(id) => symbol(ctx, id).and_then(|s| factories.get(&s).copied()),
            other => factory(ctx, other),
        };
        if matches!(
            (
                kind,
                m.static_property_name().and_then(oxc::str::JSStr::as_str)
            ),
            (Some(Factory::Server), Some("listen")) | (Some(Factory::Worker), Some("start"))
        ) && !strict(ctx, c.arguments.first().and_then(|a| a.as_expression()))
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
