use blackhole_check::engine::{Family, FileCtx, Finding, Findings, Rule, Run, registry, run};

fn mark(ctx: &FileCtx<'_>, findings: &mut Findings) {
    for (offset, _) in ctx.text.rmatch_indices('x') {
        findings.report(offset, Some("marker"));
    }
}

#[test]
fn incomplete_wide_source_retains_the_replacement_character() {
    let mut inputs = Vec::new();
    for encode in [u16::to_le_bytes, u16::to_be_bytes] {
        for (source, expected) in [
            ("const x = 1;", vec![("core/unparseable", 1, 13)]),
            (
                "// @ts-ignore",
                vec![("ts/no-comments", 1, 1), ("core/no-suppression", 1, 4)],
            ),
        ] {
            let mut bytes: Vec<_> = format!("\u{feff}{source}")
                .encode_utf16()
                .flat_map(encode)
                .collect();
            bytes.push(0x2f);
            inputs.push((bytes, expected));
        }
        let units = "\u{feff}const "
            .encode_utf16()
            .chain([0xd800])
            .chain("x=1;".encode_utf16());
        inputs.push((
            units.flat_map(encode).collect(),
            vec![("core/unparseable", 1, 7)],
        ));
    }
    for encode in [u32::to_le_bytes, u32::to_be_bytes] {
        for tail in 1..=3 {
            let mut bytes: Vec<_> = "\u{feff}const x = 1;"
                .chars()
                .flat_map(|c| encode(u32::from(c)))
                .collect();
            bytes.extend(vec![0x2f; tail]);
            inputs.push((bytes, vec![("core/unparseable", 1, 13)]));
        }
    }
    for (bytes, expected) in inputs {
        assert_eq!(
            locations(&run(registry(), [("a.ts", bytes.as_slice())])),
            expected
        );
    }
}

#[test]
fn default_export_documentation_matches_the_expression_span() {
    for (export, col) in [
        ("export default /** docs */ (x);", Some(16)),
        ("export default (/** docs */ x);", Some(17)),
        ("export default /** docs */ ((x));", Some(16)),
        ("export default /** docs */ (class {});", Some(16)),
        ("export default /** docs */ (function () {});", Some(16)),
        ("export default /** docs */ (() => {});", Some(16)),
        ("export default /** docs */ x as T;", None),
        ("export default /** docs */ (x as T);", Some(16)),
        ("export default /** docs */ x satisfies T;", None),
        ("export default /** docs */ x!;", None),
        ("export default (/** docs */ class {});", None),
        ("export default (/** docs */ (() => {}));", Some(17)),
        ("export default /** docs */ ((x) as T)!;", None),
        ("export = /** docs */ (x);", Some(10)),
    ] {
        let text = format!("type T = number; const x = 1;\n{export}");
        let hits = run(registry(), [("a.ts", text.as_bytes())]);
        assert_eq!(
            locations(&hits),
            col.map(|c| ("ts/no-comments", 2, c)).as_slice(),
            "{export}"
        );
    }
}

#[test]
fn deeply_nested_expressions_have_the_same_findings_as_shallow_ones() {
    for (open, close) in [("(", ")"), ("[", "]")] {
        let judge = |depth| {
            let text = format!(
                "// ordinary comment\nconst x = {}0{};",
                open.repeat(depth),
                close.repeat(depth)
            );
            locations(&run(registry(), [("depth.ts", text.as_bytes())]))
        };
        let expected = judge(3);
        assert_eq!(expected, [("ts/no-comments", 1, 1)]);
        assert_eq!(judge(100_000), expected);
    }
}

#[test]
fn exported_alias_chain_is_judged_in_linear_time() {
    use std::{
        fmt::Write as _,
        time::{Duration, Instant},
    };
    let mut text = String::from("/** Public value. */\ntype T0 = number;\n");
    for i in 1..4000 {
        writeln!(text, "type T{i} = T{};", i - 1).unwrap();
    }
    text.push_str("export type Last = T3999;");
    let start = Instant::now();
    assert!(run(registry(), [("aliases.ts", text.as_bytes())]).is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn merged_interfaces_are_expanded_once() {
    use std::time::{Duration, Instant};
    let text = format!(
        "/** Public tuple. */\nexport type Tuple = [{}];\n{}",
        "Merge,".repeat(8000),
        "interface Merge {}\n".repeat(8000)
    );
    let start = Instant::now();
    assert!(run(registry(), [("merge.ts", text.as_bytes())]).is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn diagnostic_points_to_the_offending_token() {
    for (path, text, line, col) in [
        (
            "a.mts",
            "type T = [first?: number,\n second: string];",
            2,
            2,
        ),
        ("a.mjs", "class C {\n  constructor() { super(); }\n}", 2, 19),
        ("a.mjs", "let x;\nlet x;", 2, 5),
        ("a.mjs", "const x=1;export {x};\nexport {x};", 2, 9),
        ("a.mjs", "tag: {\ntag: {} }", 2, 1),
        ("a.mjs", "const x = [1", 1, 13),
        ("a.mjs", "break;", 1, 1),
    ] {
        let hits = run(registry(), [(path, text.as_bytes())]);
        assert_eq!(
            locations(&hits),
            [("core/unparseable", line, col)],
            "{text}"
        );
    }
}

#[test]
fn literal_nuls_are_parse_errors_across_encodings() {
    let text = "\u{feff}// 🌑\nconst \0x = 1;";
    let encodings = [
        text.as_bytes()[3..].to_vec(),
        text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        text.chars()
            .flat_map(|c| u32::from(c).to_le_bytes())
            .collect(),
        text.chars()
            .flat_map(|c| u32::from(c).to_be_bytes())
            .collect(),
    ];
    for bytes in encodings {
        let hits = run(registry(), [("a.js", bytes.as_slice())]);
        assert_eq!(locations(&hits), [("core/unparseable", 2, 7)]);
    }
}

#[test]
fn license_lines_use_ecmascript_line_terminators() {
    for end in ["\n", "\r\n", "\r", "\u{2028}", "\u{2029}"] {
        let text = format!("/* text{end} * @license MIT{end} */ const x = 1;");
        assert!(
            run(registry(), [("a.js", text.as_bytes())]).is_empty(),
            "{end:?}"
        );
    }
}

#[test]
fn default_export_identifier_cannot_carry_docs() {
    let hits = run(
        registry(),
        [(
            "a.ts",
            b"const x = 42;\nexport default /** arbitrary */ x;".as_slice(),
        )],
    );
    assert_eq!(locations(&hits), [("ts/no-comments", 2, 16)]);
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

fn locations(findings: &[Finding]) -> Vec<(&'static str, usize, usize)> {
    findings.iter().map(|f| (f.rule, f.line, f.col)).collect()
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
fn parsed_kinds_share_the_decoded_source() {
    for ext in ["TS", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"] {
        let path = format!("file.{ext}");
        let bytes: Vec<_> = "\u{feff}\"@ts-ignore\";\n'é'; // rationale"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let found = run(registry(), [(path.as_str(), bytes.as_slice())]);
        assert_eq!(locations(&found), [("ts/no-comments", 2, 6)]);
        let broken = run(
            registry(),
            [(
                path.as_str(),
                b"\"@ts-ignore\";\nconst = ; // prose".as_slice(),
            )],
        );
        assert_eq!(
            locations(&broken),
            [("core/no-suppression", 1, 2), ("core/unparseable", 2, 7)]
        );
        assert!(broken[1].note.as_ref().is_some_and(|n| !n.is_empty()));
    }
    assert!(run(registry(), [("file.txt", b"const = ; // prose".as_slice())]).is_empty());
}

#[test]
fn regexp_syntax_and_extension_only_names_are_judged() {
    for path in ["module.js", "nested/.TS"] {
        let hits = run(registry(), [(path, b"const pattern = /(/;".as_slice())]);
        assert_eq!(
            hits.iter().map(|f| f.rule).collect::<Vec<_>>(),
            ["core/unparseable"],
            "{path}"
        );
    }
}

#[test]
fn jsx_is_read_in_every_javascript_kind() {
    for path in ["view.js", "view.mjs", "view.cjs", "view.jsx"] {
        let found = run(
            registry(),
            [(path, b"const view = <div>{1}</div>;".as_slice())],
        );
        assert_eq!(
            found.iter().map(|f| f.rule).collect::<Vec<_>>(),
            [""; 0],
            "{path}"
        );
    }
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
    let text = format!("/** {pad} */\nexport const a = 1;\n");
    let hits = run(registry(), [("a.ts", text.as_bytes())]);
    assert_eq!(
        hits.iter()
            .map(|f| (f.rule, f.note.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("core/no-suppression", Some("directive too long to read")),
            ("ts/no-comments", None)
        ]
    );
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
    for splice in [
        "\\\r",
        "\\ \n",
        "??/\t\r\n",
        "\\\u{c}\n",
        "\\\u{b}\n",
        "\\\0\n",
        "\\\n\r",
    ] {
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
fn a_comment_only_some_compilers_close_is_reported() {
    for closer in ["*??/\n/", "*\\\n\r/"] {
        let text =
            format!("#pragma GCC /* x {closer} y */ diagnostic ignored \"-Wunused-variable\"");
        let found = run(registry(), [("x.c", text.as_bytes())]);
        assert_eq!(found.len(), 1, "{closer:?}");
        assert_eq!(found[0].note.as_deref(), Some("directive too long to read"));
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
        "-S /opt/bin/python3-S -I",
    ]
    .map(|options| format!("#!/usr/bin/env {options}\n# noqa\n"));
    let lines = scripts.map(|text| {
        run(registry(), [("script", text.as_bytes())])
            .into_iter()
            .map(|f| f.line)
            .collect::<Vec<_>>()
    });
    assert_eq!(lines, [(); 5].map(|()| vec![2]));
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
