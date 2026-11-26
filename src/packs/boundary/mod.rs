use crate::engine::{Family, Findings, Gate, Rule, Run};
use crate::files::{LEDGER_PATH, OBJECTIONS_PATH};
use oxc::syntax::line_terminator::LineTerminatorSplitter;

pub const SIZE: Rule = Rule {
    id: "boundary/file-size",
    family: Family::Boundary,
    gate: Gate::Blocking,
    provenance: "Growing unrelated responsibilities in one file obscures their boundaries.",
    fix: "Split the file along its responsibilities.",
    run: Run::Lines(check),
};

fn check(path: &str, text: &str, limit: usize, findings: &mut Findings) {
    if matches!(path, LEDGER_PATH | OBJECTIONS_PATH) {
        return;
    }
    let mut lines = LineTerminatorSplitter::new(text);
    if let Some(first_excess) = lines.nth(limit) {
        let offset = first_excess.as_ptr().addr() - text.as_ptr().addr();
        let total = limit + 1 + lines.count();
        findings.report(offset, Some(&format!("{total} lines, limit {limit}")));
    }
}
