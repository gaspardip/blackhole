use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-explicit-return-types",
    family: Family::Types,
    provenance: "Port of landmines no-explicit-return-types.",
    fix: "Infer the return type. Pin a contract with a typed variable or satisfies at the return site.",
    run: Run::Ts(check),
};

use oxc::ast::{AstKind, ast::TSType};
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for node in ctx.semantic.nodes().iter() {
        let result = match node.kind() {
            AstKind::Function(n) if n.body.is_some() => n.return_type.as_deref(),
            AstKind::ArrowFunctionExpression(n) => n.return_type.as_deref(),
            _ => None,
        };
        if let Some(a) = result
            && !matches!(
                a.type_annotation,
                TSType::TSNeverKeyword(_) | TSType::TSTypePredicate(_)
            )
        {
            findings.report(a.span.start as usize, None);
        }
    }
}
