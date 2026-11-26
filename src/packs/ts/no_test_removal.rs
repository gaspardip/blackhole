use crate::engine::{self, Family, Finding, Gate, Rule, Run};
use std::io;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-test-removal",
    family: Family::Tests,
    gate: Gate::Blocking,
    provenance: "Agents delete failing tests or their assertions and report green.",
    fix: "Put the test back and make it pass. If the behaviour it covered is gone on purpose, or a test moved between files, a person approves the removal.",
    run: Run::Diff,
};

fn finding(path: &str, note: String) -> Finding {
    Finding {
        rule: RULE.id,
        gate: RULE.gate,
        path: path.to_owned(),
        line: 1,
        col: 1,
        note: Some(note),
    }
}

pub(crate) fn deleted(path: &str) -> Finding {
    finding(path, "deleted".into())
}

fn counts(path: &str, original: &str, bytes: &[u8]) -> io::Result<(usize, usize)> {
    engine::with_parsed(path, original, bytes, |ts| {
        super::no_assertionless_tests::inventory(ts).counts()
    })
}

pub(crate) fn compare(
    old: &str,
    new: &str,
    base: &[u8],
    head: &[u8],
) -> io::Result<Option<Finding>> {
    let (cases_before, assertions_before) = counts(old, old, base)?;
    let (cases_after, assertions_after) = counts(new, old, head)?;
    let notes = [
        ("test cases", cases_before, cases_after),
        ("assertions", assertions_before, assertions_after),
    ]
    .into_iter()
    .filter(|&(_, from, to)| to < from)
    .map(|(label, from, to)| format!("{label} {from} -> {to}"))
    .collect::<Vec<_>>();
    Ok((!notes.is_empty()).then(|| finding(new, notes.join(", "))))
}
