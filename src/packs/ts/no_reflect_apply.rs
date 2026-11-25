use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-reflect-apply",
    family: Family::Types,
    gate: Gate::Blocking,
    provenance: "Port of anti-slop no-reflect-apply.",
    fix: "Call the typed function directly or use its apply method with a typed receiver.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    super::reflect_method::check(ctx, findings, "apply");
}
