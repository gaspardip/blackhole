use super::dictionary_types::unknown_alias;
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::ast::AstKind;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unknown-type-aliases",
    family: Family::Types,
    provenance: "Port of anti-slop no-unknown-type-aliases.",
    fix: "Keep unknown explicit at the parsing boundary or use the parsed owner type.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for node in ctx.semantic.nodes().iter() {
        if let AstKind::TSTypeAliasDeclaration(alias) = node.kind()
            && unknown_alias(ctx, &alias.type_annotation)
        {
            findings.report(alias.id.span.start as usize, None);
        }
    }
}
