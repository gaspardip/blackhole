use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-reflect-get",
    family: Family::Types,
    gate: Gate::Blocking,
    provenance: "Port of anti-slop no-reflect-get.",
    fix: "Read the property from a typed owner using its declared key.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    super::reflect_method::check(ctx, findings, "get");
}
