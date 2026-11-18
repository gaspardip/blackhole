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
    assert_eq!(found[0].note.as_deref(), Some("directive too long to read"));
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
        (
            "long.sh",
            format!("# shellcheck shell=bash{}disable=SC2086", " ".repeat(4096)),
        ),
        ("a.sh", format!("# shellcheck {pad} disable=SC2086\n")),
        ("a.go", format!("//{}nolint\n", " ".repeat(4096))),
        ("cut.go", format!("//{}nolint\n", " ".repeat(4091))),
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

#[test]
fn utf32_directives_have_character_coordinates() {
    for encode in [u32::to_be_bytes, u32::to_le_bytes] {
        let text = "\u{feff}//\n🌑é#pragma warning disable CS0001";
        let bytes: Vec<_> = text.chars().flat_map(|c| encode(u32::from(c))).collect();
        let found = run(registry(), [("example.cs", bytes.as_slice())]);
        assert_eq!(
            found.iter().map(|f| (f.line, f.col)).collect::<Vec<_>>(),
            [(2, 3)]
        );
    }
}

#[test]
fn c_splices_inside_directive_tokens() {
    for splice in ["\\\r", "\\ \n", "??/\t\r\n"] {
        for text in [
            format!("#pragma GCC diagnostic ign{splice}ored \"-Wunused-variable\""),
            format!("#pragma GCC /* note *{splice}/ diagnostic ignored \"-Wunused-variable\""),
            format!("#pragma GCC /{splice}* note */ diagnostic ignored \"-Wunused-variable\""),
        ] {
            assert_eq!(
                run(registry(), [("x.c", text.as_bytes())]).len(),
                1,
                "{text:?}"
            );
        }
    }
}

#[test]
fn an_attribute_that_never_closes_is_reported() {
    let text = format!("{}[doc = \"open\nfn f() {{}}\n", '#');
    let found = run(registry(), [("a.rs", text.as_bytes())]);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].line, found[0].col), (1, 1));
    assert_eq!(found[0].note.as_deref(), Some("unreadable attribute"));
}

#[test]
fn window_cut_inside_attribute_token() {
    let text = format!("{}[{}allow(dead_code)]", '#', " ".repeat(4091));
    let found = run(registry(), [("x.rs", text.as_bytes())]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].note.as_deref(), Some("directive too long to read"));
}

#[test]
fn env_options_can_name_multiple_interpreters() {
    let scripts = [
        "-Spython3",
        "--split-string=python3",
        "-S -u /bin/bash python3",
        "-iSpython3",
    ]
    .map(|options| format!("#!/usr/bin/env {options}\n# noqa\n"));
    let lines = scripts.map(|text| {
        run(registry(), [("script", text.as_bytes())])
            .into_iter()
            .map(|f| f.line)
            .collect::<Vec<_>>()
    });
    assert_eq!(lines, [vec![2], vec![2], vec![2], vec![2]]);
    // An interpreter's own name can hold the letter that ends the options.
    for options in ["-S BASH -e", "-iSBASH"] {
        let text = format!("#!/usr/bin/env {options}\n# shellcheck disable=SC2086\n");
        let found = run(registry(), [("script", text.as_bytes())]);
        assert_eq!(found.len(), 1, "{options}");
    }
}

#[test]
fn long_c_pragma_comment_is_reported() {
    let text = format!(
        "#pragma GCC/*{}*/diagnostic ignored \"-Wunused-variable\"",
        "x".repeat(4097)
    );
    let found = run(registry(), [("long.c", text.as_bytes())]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].note.as_deref(), Some("directive too long to read"));
}

#[test]
fn a_byte_order_mark_is_a_gap_and_not_part_of_a_name() {
    let found = run(
        registry(),
        [(
            "a.cs",
            "#pragma\u{feff}warning disable CS0168\n[Suppress\u{feff}Message(\"x\", \"y\")]\n#pragma\u{200b} warning\u{200b}\u{feff}\u{200b}disable CS0168\n#nullable\u{ad} disable\n"
                .as_bytes(),
        )],
    );
    assert_eq!(
        found.iter().map(|f| (f.line, f.col)).collect::<Vec<_>>(),
        [(1, 1), (2, 2), (3, 1), (4, 1)]
    );
}
