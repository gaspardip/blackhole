use crate::engine::{Family, Gate, Rule, Run};

pub const SIZE: Rule = Rule {
    id: "boundary/file-size",
    family: Family::Boundary,
    gate: Gate::Blocking,
    provenance: "Growing unrelated responsibilities in one file obscures their boundaries.",
    fix: "Split the file along its responsibilities.",
    run: Run::Lines,
};
