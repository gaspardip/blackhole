use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::{
    AstKind,
    ast::{AssignmentTarget, Expression, IdentifierReference, ObjectPropertyKind, PropertyKey},
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
fn symbol(ctx: &TsCtx<'_>, id: &IdentifierReference<'_>) -> Option<SymbolId> {
    ctx.semantic
        .scoping()
        .get_reference(id.reference_id.get()?)
        .symbol_id()
}
fn factory(ctx: &TsCtx<'_>, e: &Expression<'_>) -> Option<u8> {
    let Expression::CallExpression(c) = e else {
        return None;
    };
    let Expression::Identifier(id) = &c.callee else {
        return None;
    };
    let decl = ctx.semantic.symbol_declaration(symbol(ctx, id)?);
    let AstKind::ImportSpecifier(s) = decl.kind() else {
        return None;
    };
    let AstKind::ImportDeclaration(i) = ctx.semantic.nodes().parent_kind(decl.id()) else {
        return None;
    };
    if !i
        .source
        .value
        .as_str()
        .is_some_and(|s| s == "msw" || s.starts_with("msw/"))
    {
        return None;
    }
    match s.imported.name().as_str() {
        "setupServer" => Some(1),
        "setupWorker" => Some(2),
        _ => None,
    }
}
fn strict(opts: Option<&Expression<'_>>) -> bool {
    matches!(opts,Some(Expression::ObjectExpression(o)) if o.properties.iter().any(|p|matches!(p,ObjectPropertyKind::ObjectProperty(p) if !p.computed && matches!(&p.key,PropertyKey::StaticIdentifier(k) if k.name=="onUnhandledRequest") && (matches!(&p.value,Expression::StringLiteral(s) if s.value=="error")||matches!(&p.value,Expression::FunctionExpression(_)|Expression::ArrowFunctionExpression(_))))))
}
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut factories = HashMap::<SymbolId, u8>::new();
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
            *factories.entry(symbol).or_default() |= kind;
        }
    }
    for node in ctx.semantic.nodes().iter() {
        let AstKind::CallExpression(c) = node.kind() else {
            continue;
        };
        let Expression::StaticMemberExpression(m) = &c.callee else {
            continue;
        };
        let kind = match &m.object {
            Expression::Identifier(id) => symbol(ctx, id).and_then(|s| factories.get(&s).copied()),
            other => factory(ctx, other),
        };
        if matches!(
            (kind, m.property.name.as_str()),
            (Some(1), "listen") | (Some(2), "start")
        ) && !strict(c.arguments.first().and_then(|a| a.as_expression()))
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
