use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::ast::AstKind;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-explicit-any",
    family: Family::Types,
    gate: Gate::Blocking,
    provenance: "Agents reach for any to make a type error go away.",
    fix: "Name the real type. When the value is truly unknown, parse it at the boundary into a named type.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for node in ctx.semantic.nodes().iter() {
        if let AstKind::TSAnyKeyword(keyword) = node.kind() {
            findings.report(keyword.span.start as usize, None);
        }
    }
}
