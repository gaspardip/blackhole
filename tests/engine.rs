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

fn places(text: &[u8]) -> Vec<(usize, usize)> {
    let rule = Rule {
        id: "a",
        family: Family::Judge,
        provenance: "test",
        fix: "fix",
        run: Run::File(mark),
    };
    run(&[rule], [("a", text)])
        .iter()
        .map(|f| (f.line, f.col))
        .collect()
}

#[test]
fn every_line_break_counts_once() {
    assert_eq!(
        places("x\rx\r\nx\u{2028}x\u{2029}x".as_bytes()),
        [(1, 1), (2, 1), (3, 1), (4, 1), (5, 1)]
    );
}

#[test]
fn wide_encodings_are_decoded() {
    assert_eq!(places(b"\xff\xfea\0\n\0x\0"), [(2, 1)]);
    assert_eq!(places(b"\xfe\xff\0a\0\n\0x"), [(2, 1)]);
    assert_eq!(places(b"\0\0\xfe\xff\0\0\0\n\0\0\0x"), [(2, 1)]);
}

#[test]
fn an_attribute_too_long_to_read_is_reported() {
    let pad = "y".repeat(5000);
    let text = format!("{}[/*{pad}*/allow(dead_code)]\nfn f() {{}}\n", '#');
    let found = run(registry(), [("a.rs", text.as_bytes())]);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].line, found[0].col), (1, 1));
    assert_eq!(found[0].note.as_deref(), Some("attribute too long to read"));
    let short = format!("{}[/*{}*/allow(dead_code)]", '#', "y".repeat(50));
    assert_eq!(run(registry(), [("a.rs", short.as_bytes())])[0].note, None);
}

#[test]
fn a_directive_too_long_to_read_is_reported() {
    let pad = "y".repeat(5000);
    for (path, text) in [
        (
            "a.c",
            format!("#pragma GCC /*{pad}*/ diagnostic ignored \"-Wunused\"\n"),
        ),
        ("a.sh", format!("# shellcheck {pad} disable=SC2086\n")),
    ] {
        let found = run(registry(), [(path, text.as_bytes())]);
        assert_eq!(found.len(), 1, "{path}");
        assert_eq!(
            found[0].note.as_deref(),
            Some("directive too long to read"),
            "{path}"
        );
    }
    // A comment that never closes before the end of the file hides nothing.
    assert!(run(registry(), [("a.py", &b"# /* notes"[..])]).is_empty());
}
