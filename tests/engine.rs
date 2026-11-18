use blackhole_check::engine::{Family, FileCtx, Findings, Rule, Run, registry, run};

fn mark(ctx: &FileCtx<'_>, findings: &mut Findings) {
    for (offset, _) in ctx.text.rmatch_indices('x') {
        findings.report(offset, Some("marker"));
    }
}

#[test]
fn coordinates_and_sort_order() {
    let rules = ["b", "a"].map(|id| Rule {
        id,
        family: Family::Judge,
        provenance: "test",
        fix: "fix",
        run: Run::File(mark),
    });
    let found = run(&rules, [("z", &b"x"[..]), ("a", "x\r\nxéx".as_bytes())]);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.path.as_str(), f.line, f.col, f.rule, f.note.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("a", 1, 1, "a"),
            ("a", 1, 1, "b"),
            ("a", 2, 1, "a"),
            ("a", 2, 1, "b"),
            ("a", 2, 3, "a"),
            ("a", 2, 3, "b"),
            ("z", 1, 1, "a"),
            ("z", 1, 1, "b")
        ]
        .map(|(p, l, c, r)| (p, l, c, r, Some("marker")))
    );
}

#[test]
fn registry_sorted_and_unique() {
    let rules = registry();
    assert!(rules.iter().any(|r| r.id == "core/no-suppression"));
    assert!(rules.windows(2).all(|w| w[0].id < w[1].id));
}
