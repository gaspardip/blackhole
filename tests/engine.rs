use blackhole_check::engine::{Family, FileCtx, Finding, Findings, Rule, Run, registry, run};

fn mark(ctx: &FileCtx<'_>, findings: &mut Findings) {
    for (offset, _) in ctx.text.rmatch_indices('x') {
        findings.report(offset, Some("marker"));
    }
}

#[test]
fn malformed_units_are_reported_even_inside_allowed_syntax() {
    let cases = [
        ("// WARNING:", "", 12),
        ("const s = \"", "\";", 12),
        ("const t = `", "`;", 12),
        ("const x = 1;", "", 13),
    ];
    for (prefix, suffix, column) in cases {
        let text = format!("\u{feff}{prefix}");
        let check = |bytes: &[u8]| {
            assert_eq!(
                locations(&run(registry(), [("a.ts", bytes)])),
                [("core/unparseable", 1, column)],
                "{prefix}"
            );
        };
        for encode in [u16::to_le_bytes, u16::to_be_bytes] {
            for bad in [0xd800, 0xdc00] {
                let bytes: Vec<_> = text
                    .encode_utf16()
                    .chain([bad])
                    .chain(suffix.encode_utf16())
                    .flat_map(encode)
                    .collect();
                check(&bytes);
            }
            if suffix.is_empty() {
                let mut bytes: Vec<_> = text.encode_utf16().flat_map(encode).collect();
                bytes.push(0x2f);
                check(&bytes);
            }
        }
        for encode in [u32::to_le_bytes, u32::to_be_bytes] {
            for bad in [0xd800, 0x0011_0000] {
                let bytes: Vec<_> = text
                    .chars()
                    .map(u32::from)
                    .chain([bad])
                    .chain(suffix.chars().map(u32::from))
                    .flat_map(encode)
                    .collect();
                check(&bytes);
            }
            if suffix.is_empty() {
                for count in 1..=3 {
                    let mut bytes: Vec<_> =
                        text.chars().flat_map(|c| encode(u32::from(c))).collect();
                    bytes.extend(vec![0x2f; count]);
                    check(&bytes);
                }
            }
        }
    }
    for encode in [u16::to_le_bytes, u16::to_be_bytes] {
        let mut bytes: Vec<_> = "\u{feff}const "
            .encode_utf16()
            .chain([0xd800])
            .flat_map(encode)
            .collect();
        bytes.push(0x2f);
        assert_eq!(
            locations(&run(registry(), [("a.ts", bytes.as_slice())])),
            [("core/unparseable", 1, 7)]
        );
    }
}

#[test]
fn valid_unicode_and_raw_encoding_verdicts_are_preserved() {
    let valid = "\u{feff}const s = '🌑�';";
    for bytes in [
        valid.as_bytes().to_vec(),
        valid.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        valid.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        valid
            .chars()
            .flat_map(|c| u32::from(c).to_be_bytes())
            .collect(),
        valid
            .chars()
            .flat_map(|c| u32::from(c).to_le_bytes())
            .collect(),
    ] {
        assert!(run(registry(), [("valid.ts", bytes.as_slice())]).is_empty());
    }
    assert_eq!(
        locations(&run(
            registry(),
            [("quoted.ts", b"const x=\"\xff\";".as_slice())]
        )),
        [("core/unparseable", 1, 10)]
    );
    for path in ["a.ts", "a.js"] {
        assert_eq!(
            locations(&run(registry(), [(path, b"'\0\xff'".as_slice())])),
            [("core/unparseable", 1, 3)]
        );
    }
    for path in ["a.py", "a.rs"] {
        assert!(run(registry(), [(path, b"'\xff'".as_slice())]).is_empty());
    }
    assert_eq!(
        locations(&run(
            registry(),
            [("a.ts", b"// @ts-ignore\xff".as_slice())]
        )),
        [("core/no-suppression", 1, 4), ("core/unparseable", 1, 14)]
    );
    assert_eq!(
        locations(&run(registry(), [("a.py", b"# noqa\xff".as_slice())])),
        [("core/no-suppression", 1, 1)]
    );
}

#[test]
fn documentation_and_directives_use_javascript_whitespace() {
    for gap in ['\u{85}', '\u{feff}', '\u{2028}'] {
        for (source, allowed) in [
            ("export default (/** docs */GAPclass {});", gap != '\u{85}'),
            ("export = (/** docs */GAPclass {});", false),
            ("/** docs */GAPexport class C {}", gap != '\u{85}'),
            ("/* @jsxGAPh */", gap != '\u{85}'),
            ("/* @ts-checkGAPexplanation */", gap != '\u{85}'),
            ("/* #GAPsourceURL=x */", gap != '\u{85}'),
            ("/* /GAP<reference types='node' /> */", gap != '\u{85}'),
            ("/*\nGAP* @license MIT\n */", gap != '\u{85}'),
        ] {
            let text = source.replace("GAP", &gap.to_string());
            let hits = run(registry(), [("a.ts", text.as_bytes())]);
            let column = source.find('/').unwrap() + 1;
            let expected = (!allowed).then_some(("ts/no-comments", 1, column));
            assert_eq!(locations(&hits), expected.as_slice(), "{text:?}");
        }
    }
    for (gap, allowed) in [('\u{85}', true), ('\u{feff}', false)] {
        let text = format!("// WARNING:{gap}");
        let hits = run(registry(), [("a.ts", text.as_bytes())]);
        assert_eq!(
            locations(&hits),
            (!allowed).then_some(("ts/no-comments", 1, 1)).as_slice()
        );
    }
}

#[test]
fn suppression_after_literal_nul_keeps_its_source_column() {
    let wide: Vec<_> = "\u{feff}const s = \"🌑\0c8 ignore\";"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    for (path, bytes, expected) in [
        (
            "literal.ts",
            b"const s = \"\0c8 ignore\";".as_slice(),
            vec![("core/unparseable", 1, 12), ("core/no-suppression", 1, 13)],
        ),
        (
            "split.py",
            "é\0 # no\0qa".as_bytes(),
            vec![("core/no-suppression", 1, 4)],
        ),
        (
            "split.rs",
            b"\0  #[al\0low(dead_code)]",
            vec![("core/no-suppression", 1, 4)],
        ),
        (
            "two.py",
            b"\0\0# noqa\0",
            vec![("core/no-suppression", 1, 3)],
        ),
        (
            "lines.py",
            b"a\r\0\nb\n# noqa",
            vec![("core/no-suppression", 4, 1)],
        ),
        (
            "start.ts",
            b"\0\"v8 ignore\";",
            vec![("core/unparseable", 1, 1), ("core/no-suppression", 1, 3)],
        ),
        (
            "end.ts",
            b"const a = 1;\0",
            vec![("core/unparseable", 1, 13)],
        ),
        (
            "wide.ts",
            wide.as_slice(),
            vec![("core/unparseable", 1, 13), ("core/no-suppression", 1, 14)],
        ),
    ] {
        assert_eq!(
            locations(&run(registry(), [(path, bytes)])),
            expected,
            "{path}"
        );
    }
}

#[test]
fn incomplete_wide_source_retains_the_replacement_character() {
    let mut inputs = Vec::new();
    for encode in [u16::to_le_bytes, u16::to_be_bytes] {
        for (source, expected) in [
            ("const x = 1;", vec![("core/unparseable", 1, 13)]),
            // Malformed UTF-16 prevents ts/no-comments from receiving a parsed context.
            (
                "// @ts-ignore",
                vec![("core/no-suppression", 1, 4), ("core/unparseable", 1, 14)],
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
        let mut hits = run(registry(), [("a.ts", text.as_bytes())]);
        // The other TypeScript rules judge these wrappers themselves.
        hits.retain(|f| f.rule == "ts/no-comments" || !f.rule.starts_with("ts/"));
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
fn deeply_parenthesized_parameter_reports_the_keyword() {
    let prefix = "declare function f(value: ";
    let depth = 100_000;
    let source = format!(
        "{prefix}{}unknown{}): void;",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    assert_eq!(
        locations(&run(registry(), [("parameter.ts", source.as_bytes())])),
        [("ts/no-unknown-parameters", 1, prefix.len() + depth + 1)]
    );
}

#[test]
fn deeply_nested_array_reports_its_element_type() {
    let prefix = "type Matrix = ";
    let source = format!("{prefix}any{};", "[]".repeat(100_000));
    assert_eq!(
        locations(&run(registry(), [("matrix.ts", source.as_bytes())])),
        [("ts/no-explicit-any", 1, prefix.len() + 1)]
    );
}

#[test]
fn inherited_interface_members_determine_dictionary_safety() {
    let cases = [
        ("interface Base {}", "Base", true),
        ("interface A {} interface Base extends A {}", "Base", true),
        ("interface A {} interface B {}", "A, B", true),
        ("interface A {} interface B { id: string }", "A, B", false),
        (
            "interface Base {} interface Base { value: string }",
            "Base",
            false,
        ),
        ("class Base {}", "Base", false),
        ("import type { Base } from './other';", "Base", false),
        ("interface Base<T> { value: T }", "Base<string>", false),
        ("", "Missing", false),
    ];
    for (definitions, parents, empty) in cases {
        let source = format!(
            "{definitions}\ninterface Model extends {parents} {{}}\ntype Dict = Record<string, Model>;"
        );
        let rule = registry()
            .iter()
            .find(|r| r.id == "ts/no-unsafe-dictionary-type")
            .unwrap();
        let hits = run(
            std::slice::from_ref(rule),
            [("interface.ts", source.as_bytes())],
        );
        let expected = if empty {
            vec![(rule.id, 3, 13)]
        } else {
            vec![]
        };
        assert_eq!(locations(&hits), expected, "{source}");
    }
}

#[test]
fn only_type_contributors_determine_interface_emptiness() {
    let rule = registry()
        .iter()
        .find(|r| r.id == "ts/no-unsafe-dictionary-type")
        .unwrap();
    for (decls, col) in [
        ("interface Model {}\nenum Model { Id }", 6),
        ("enum Model { Id }\ninterface Model {}", 11),
    ] {
        let source = format!("{decls}\ntype Dict = Record<string, Model>;");
        assert_eq!(
            locations(&run(registry(), [("merged.ts", source.as_bytes())])),
            [("core/unparseable", 2, col)]
        );
    }
    for (other, empty) in [
        ("class Model { id = 0; }", false),
        ("declare class Model { id: number; }", false),
        ("namespace Model { export const id = 0; }", true),
        ("const Model = 1;", true),
        ("let Model = 1;", true),
        ("var Model = 1;", true),
        ("declare const Model: number;", true),
        ("function Model() {}", true),
        ("import type { Model } from './other';", false),
        ("interface Model extends Base {}", false),
        ("interface Model { id: number }", false),
        ("interface Model {}", true),
    ] {
        let declarations = ["interface Model {}", other];
        for [first, second] in [declarations, [other, declarations[0]]] {
            let source = format!("{first}\n{second}\ntype Dict = Record<string, Model>;");
            let hits = run(registry(), [("merged.ts", source.as_bytes())]);
            assert!(
                hits.iter().all(|h| h.rule != "core/unparseable"),
                "{source}"
            );
            let actual: Vec<_> = hits
                .iter()
                .filter(|f| f.rule == rule.id)
                .map(|f| (f.line, f.col))
                .collect();
            let expected = if empty { vec![(3, 13)] } else { vec![] };
            assert_eq!(actual, expected, "{source}");
        }
    }
}

#[test]
fn guard_evidence_uses_only_an_identifiers_own_annotation() {
    let rule = registry()
        .iter()
        .find(|r| r.id == "ts/no-known-value-widening")
        .unwrap();
    let guard = "function guard(v: unknown): v is string { return true; }";
    let check = |body: &str| {
        let src = format!("{guard}\n{body}");
        run(std::slice::from_ref(rule), [("case.ts", src.as_bytes())])
    };
    let patterns = [
        "declare const input: object;\nconst { value } = input as { value: unknown };\nguard(value);",
        "type Foo = { id: number };\ndeclare const src: { a: Foo };\nlet { a }: { a: Foo } = src;\na = { id: 1 };",
    ];
    for body in patterns {
        assert!(check(body).is_empty(), "{body}");
    }
    for parameter in [
        "...[value]: [unknown]",
        "[value]: [unknown]",
        "{ value }: { value: unknown }",
        "[{ value }]: [{ value: unknown }]",
        "[value = 0]: [unknown]",
        "[...value]: unknown[]",
    ] {
        assert!(
            check(&format!("function wrap({parameter}) {{ guard(value); }}")).is_empty(),
            "{parameter}"
        );
    }
    for binding in [
        "[value]: [unknown]",
        "{ value }: { value: unknown }",
        "[{ value }]: [{ value: unknown }]",
        "[value = 0]: [unknown]",
        "[...value]: unknown[]",
    ] {
        assert!(
            check(&format!(
                "declare const source: object; const {binding} = source; guard(value);"
            ))
            .is_empty(),
            "{binding}"
        );
    }
    for parameter in ["value: string = ''", "...value: string[]"] {
        let body = format!("function wrap({parameter}) {{ guard(value); }}");
        assert_eq!(
            locations(&check(&body)),
            [(rule.id, 2, body.find("guard(value)").unwrap() + 7)]
        );
    }
}

#[test]
fn type_only_imports_leave_local_value_evidence_visible() {
    let rule = registry()
        .iter()
        .find(|r| r.id == "ts/no-widen-then-assert")
        .unwrap();
    let check = |source: &str| {
        run(
            std::slice::from_ref(rule),
            [("value.ts", source.as_bytes())],
        )
    };
    for declaration in [
        "import type { value } from './types';",
        "import type value = require('./types');",
        "import type value from './types';",
        "import type * as value from './types';",
        "import { type X as value } from './types';",
        "export type { value } from './types';",
    ] {
        let source = format!(
            "{declaration}\nconst value: unknown = {{ id: 1 }};\nconst picked = value as {{ id: number }};"
        );
        assert_eq!(
            locations(&check(&source)),
            [(rule.id, 3, 16)],
            "{declaration}"
        );
    }
    let ambient = "import type { value } from './types';\ndeclare const value: unknown;\nconst picked = value as { id: number };";
    assert!(check(ambient).is_empty());
}

#[test]
fn alias_verdicts_follow_declarations_in_every_order() {
    let cases: &[(&str, &[(&str, usize)])] = &[
        (
            "ts/no-unsafe-dictionary-type",
            &[
                ("type Id<T> = T;", 0),
                ("type Dict<U> = Id<Record<string, Id<U>>>;", 0),
                ("type Broken = Dict<unknown>;", 15),
                ("type X = Id<Id<Record<string, unknown>>>;", 10),
            ],
        ),
        (
            "ts/no-unknown-type-aliases",
            &[
                ("type Id<T> = T;", 0),
                ("type Box<T = unknown> = Id<T>;", 0),
                ("type Outer<T> = Id<Box>;", 6),
                ("type Seed = Box;", 6),
                ("type Check = Outer<string>;", 6),
            ],
        ),
        (
            "ts/no-unknown-type-aliases",
            &[
                ("type L<T> = T | L<T[]>;", 0),
                ("type A<T> = B<T[]>;", 0),
                ("type B<T> = A<T[]>;", 0),
                ("type Json = string | Json[] | { [k: string]: Json };", 0),
                (
                    "type DeepPartial<T> = { [K in keyof T]?: DeepPartial<T[K]> };",
                    0,
                ),
                ("type L1 = L<unknown>;", 6),
                ("type L2 = L<string>;", 0),
                ("type M = A<string>;", 0),
                ("type D = DeepPartial<Json>;", 0),
            ],
        ),
        (
            "ts/no-unsafe-dictionary-type",
            &[
                ("type A = B | unknown;", 0),
                ("type B = A & object;", 0),
                ("type C = D | E;", 0),
                ("type D = E & object;", 0),
                ("type E = C | unknown;", 0),
                ("type X = Record<string, B>;", 10),
                ("type Y = Record<string, D>;", 10),
            ],
        ),
    ];
    for &(id, declarations) in cases {
        let rule = registry().iter().find(|r| r.id == id).unwrap();
        let mut lines = declarations.to_vec();
        for order in 0..3 {
            match order {
                1 => lines.reverse(),
                2 => {
                    lines.reverse();
                    lines.rotate_left(1);
                }
                _ => {}
            }
            let source = lines.iter().map(|&(s, _)| s).collect::<Vec<_>>().join("\n");
            let expected: Vec<_> = lines.iter().copied().filter(|&(_, col)| col > 0).collect();
            let actual: Vec<_> = run(std::slice::from_ref(rule), [("case.ts", source.as_bytes())])
                .into_iter()
                .map(|f| (lines[f.line - 1].0, f.col))
                .collect();
            assert_eq!(actual, expected, "{source}");
        }
    }
}

fn timed_alias_findings(source: &str, seconds: u64) -> Vec<Finding> {
    let start = std::time::Instant::now();
    let findings = run(registry(), [("aliases.ts", source.as_bytes())]);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(seconds),
        "{:?}",
        start.elapsed()
    );
    findings
}

#[test]
fn long_alias_chain_and_wide_union_fit_the_budget() {
    use std::fmt::Write as _;
    let mut chain = String::from("type A0 = string;\n");
    for i in 1..100_000 {
        writeln!(chain, "type A{i} = A{};", i - 1).unwrap();
        if i == 99 {
            let members = vec!["A99"; 5000].join(" | ");
            assert!(timed_alias_findings(&format!("{chain}type Wide = {members};"), 3).is_empty());
        }
    }
    assert!(timed_alias_findings(&chain, 3).is_empty());
}

#[test]
fn shared_union_members_fit_the_budget_with_and_without_a_cycle() {
    use std::fmt::Write as _;
    for (depth, leaf) in [
        (27, "string".to_owned()),
        (20, "A20".to_owned()),
        (24, "A24".to_owned()),
    ] {
        let mut source = format!("type A0 = {leaf};\n");
        for i in 1..=depth {
            let prev = i - 1;
            writeln!(source, "type A{i} = A{prev} | A{prev};").unwrap();
        }
        assert!(timed_alias_findings(&source, 1).is_empty());
    }
}

#[test]
fn recursive_node_types_propagate_unknown_to_every_alias() {
    use std::fmt::Write as _;
    let mut source = String::from("type Leaf = unknown;\n");
    for i in 0..40 {
        let children: Vec<_> = [1, 3, 7, 13]
            .map(|offset| format!("N{}", (i + offset) % 40))
            .into();
        let leaf = if i == 0 { " | Leaf" } else { "" };
        writeln!(source, "type N{i} = {}{leaf};", children.join(" | ")).unwrap();
    }
    let hits = timed_alias_findings(&source, 1);
    let expected: Vec<_> = (1..=41)
        .map(|i| ("ts/no-unknown-type-aliases", i, 6))
        .collect();
    assert_eq!(locations(&hits), expected);
    assert!(hits.iter().all(|f| f.note.is_none()));
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
fn export_aliases_expand_each_root_binding_once() {
    use std::time::{Duration, Instant};
    let aliases = (0..4000)
        .map(|i| format!("M as A{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let text = format!(
        "/** Public contract. */\n{}export {{ {aliases} }};",
        "interface M {}\n".repeat(4000)
    );
    let start = Instant::now();
    assert!(run(registry(), [("aliases.ts", text.as_bytes())]).is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_gap_after_a_comment_is_scanned_once() {
    use std::{
        fmt::Write as _,
        time::{Duration, Instant},
    };
    let mut text = format!("/** Public value. */{}", " ".repeat(100_000));
    for i in 0..2000 {
        writeln!(text, "export const a{i} = 0;").unwrap();
    }
    let start = Instant::now();
    assert!(run(registry(), [("gap.ts", text.as_bytes())]).is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_lone_valid_replacement_character_locates_the_binary_verdict() {
    for (text, at) in [
        ("const a = 1;\nconst b = \u{fffd};\n", (2, 11)),
        ("const a = '\u{fffd}';\nconst b = \u{fffd};\n", (1, 1)),
    ] {
        assert_eq!(
            locations(&run(registry(), [("a.ts", text.as_bytes())])),
            [("core/unparseable", at.0, at.1)]
        );
    }
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
fn shared_generic_types_keep_their_argument_and_query_results_separate() {
    let source = b"type Wrapped<T> = { item: T };\ntype Identity<T> = T;\ntype User = Wrapped<string>;\ntype Input = Identity<unknown>;\ntype Count = Identity<number>;\ndeclare function fetch(): Promise<Input>;\ndeclare function count(): Promise<Count>;\n";
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let hits = run(registry(), [("contracts.ts", source.as_slice())]);
        send.send(
            hits.into_iter()
                .map(|h| (h.rule, h.line, h.col))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    });
    let hits = receive
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    worker.join().unwrap();
    assert_eq!(
        hits,
        [
            ("ts/no-unknown-type-aliases", 4, 6),
            ("ts/no-unknown-returns", 6, 27)
        ]
    );
}

#[test]
fn a_deep_const_chain_keeps_its_original_type_evidence() {
    use std::fmt::Write;
    let mut src = String::from("const item0 = {};\n");
    for i in 1..=10_000 {
        writeln!(src, "const item{i} = item{};", i - 1).unwrap();
    }
    src.push_str("const widened: unknown = item10000;\nconst user = widened as User;\n");
    let hits = run(registry(), [("values.ts", src.as_bytes())]);
    assert_eq!(
        hits.iter()
            .map(|h| (h.rule, h.line, h.col))
            .collect::<Vec<_>>(),
        [
            ("ts/no-known-value-widening", 10_002, 26),
            ("ts/no-widen-then-assert", 10_003, 14)
        ]
    );
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
