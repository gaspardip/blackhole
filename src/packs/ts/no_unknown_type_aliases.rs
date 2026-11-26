use super::dictionary_types::{Resolution, report};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::ast::AstKind;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unknown-type-aliases",
    family: Family::Types,
    gate: Gate::Blocking,
    provenance: "Port of anti-slop no-unknown-type-aliases.",
    fix: "Keep unknown explicit at the parsing boundary or use the parsed owner type.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    check_aliases(ctx, findings, &Resolution::default());
}

pub(super) fn check_aliases(ctx: &TsCtx<'_>, findings: &mut Findings, resolution: &Resolution) {
    for node in ctx.nodes() {
        resolution.begin_candidate();
        if let AstKind::TSTypeAliasDeclaration(alias) = node.kind() {
            report(
                findings,
                alias.id.span.start as usize,
                resolution.contains_unknown(ctx, &alias.type_annotation),
            );
        }
    }
}
