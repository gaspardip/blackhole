#[path = "common/work.rs"]
mod work;
use work::{assert_growth, assert_work};

use blackhole_check::engine::{registry, run, run_with_work};
use std::{
    fmt::Write,
    time::{Duration, Instant},
};

fn source(shape: &str, n: usize) -> String {
    let mut text = String::new();
    match shape {
        "overwrites" => {
            text.push_str("let x=vi;");
            for _ in 0..n {
                text.push_str("x=vi;x=custom;");
            }
            text.push_str("x.mock('client');");
        }
        "object-spreads" | "aliased-spreads" | "array-spreads" => {
            let array = shape == "array-spreads";
            let (left, right) = if array { ("[", "]") } else { ("{", "}") };
            writeln!(text, "const a0={left}{right};").unwrap();
            for i in 1..=n {
                let prev = i - 1;
                let parent = if shape == "aliased-spreads" {
                    writeln!(text, "const p{i}=a{prev};").unwrap();
                    format!("p{i}")
                } else {
                    format!("a{prev}")
                };
                writeln!(text, "const a{i}={left}...{parent},...{parent}{right};").unwrap();
            }
            if array {
                writeln!(text, "vi.setConfig(...a{n});").unwrap();
            } else {
                writeln!(text, "vi.setConfig(a{n});").unwrap();
            }
        }
        "chain" | "chaincalls" => {
            let links = if shape == "chain" { n } else { n / 2 };
            text.push_str("const a0=test;\n");
            for i in 1..links {
                writeln!(text, "const a{i}=a{};", i - 1).unwrap();
            }
            for _ in 0..if shape == "chain" { 1 } else { n / 2 } {
                writeln!(text, "a{}('x',()=>expect(read()).toBe(answer));", links - 1).unwrap();
            }
        }
        "callchain" => {
            text.push_str("test('x',()=>expect(read())");
            for _ in 0..n {
                text.push_str(".and()");
            }
            text.push_str(".toBe(answer));");
        }
        "fnhelper" => {
            text.push_str("function helper(i:number){return i;} test('x',()=>{");
            for i in 0..n {
                writeln!(text, "expect(helper({i})).toBe({i});").unwrap();
            }
            text.push_str("});");
        }
        _ => {
            text.push_str(if shape == "imported" {
                "import {test as t,expect as e} from 'vitest';"
            } else {
                "let t=test;let e=expect;"
            });
            for i in 0..n {
                writeln!(text, "t('x',()=>e(read({i})).toBe({i}));").unwrap();
            }
        }
    }
    text
}

#[test]
fn resolves_large_files() {
    // Parsing a long call chain needs a larger stack before resolution runs.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(check_shapes)
        .unwrap()
        .join()
        .unwrap();
}

fn check_shapes() {
    for (shape, units_per_byte) in [
        ("object-spreads", 22),
        ("aliased-spreads", 24),
        ("array-spreads", 17),
        ("overwrites", 38),
        ("imported", 55),
        ("letalias", 54),
        ("fnhelper", 47),
        ("callchain", 86),
        ("chaincalls", 33),
        ("chain", 17),
    ] {
        let text = source(shape, 10_000);
        let (hits, work) = run_with_work(registry(), "large.test.ts", text.as_bytes());
        assert!(hits.is_empty(), "{shape}");
        assert_work(work, text.len(), units_per_byte, shape);
    }
}

#[test]
fn sdk_transport_spreads_scale_with_distinct_objects() {
    let mut text =
        String::from("import Knock from '@knocklabs/node';\nconst opts0 = { name: 'sample' };\n");
    for i in 1..=23 {
        let prev = i - 1;
        writeln!(text, "const opts{i} = {{ ...opts{prev}, ...opts{prev} }};").unwrap();
    }
    text.push_str("new Knock(opts23);");
    let (hits, work) = run_with_work(registry(), "case.test.ts", text.as_bytes());
    assert_eq!(
        hits.iter().map(|hit| hit.rule).collect::<Vec<_>>(),
        Vec::<&str>::new()
    );
    assert_work(work, text.len(), 14, "SDK option spreads");
}

#[test]
fn sdk_transport_spreads_keep_overwrite_precedence() {
    for (options, expected) in [
        ("{ ...fakeOptions, ...realOptions }", 0),
        ("{ ...realOptions, ...fakeOptions }", 1),
        ("{ ...fakeOptions, ...readOptions() }", 0),
        ("{ ...readOptions(), ...fakeOptions }", 1),
        ("{ ...fakeOptions, [readKey()]: fetch }", 1),
        ("{ ...realOptions, [readKey()]: fake }", 1),
    ] {
        let source = format!(
            "import Knock from '@knocklabs/node';\nconst fake = () => new Response('sample');\nconst fakeOptions = {{ fetch: fake }};\nconst realOptions = {{ fetch }};\nnew Knock({options});"
        );
        let hits = run(registry(), [("case.test.ts", source.as_bytes())]);
        assert_eq!(
            hits.iter().map(|hit| hit.rule).collect::<Vec<_>>(),
            vec!["ts/no-test-doubles"; expected],
            "{options}"
        );
    }
}

#[test]
fn repeated_applications_share_their_default_bindings() {
    for (parameters, uses) in [(2000, 48), (4000, 100)] {
        let mut source = String::from("type A<T0 = unknown");
        for i in 1..parameters {
            write!(source, ", T{i} = T0").unwrap();
        }
        source.push_str("> = T0;\n");
        for i in 0..uses {
            writeln!(source, "function f{i}(x: A) {{}}").unwrap();
        }
        let (hits, work) = run_with_work(registry(), "defaults.ts", source.as_bytes());
        assert_work(work, source.len(), 28, "default bindings");
        assert_eq!(hits.len(), uses);
        for (index, h) in hits.iter().enumerate() {
            assert_eq!(
                (h.rule, h.line, h.note.as_deref()),
                ("ts/no-unknown-parameters", index + 2, None)
            );
        }
    }
}

#[test]
fn large_declarations_and_repeated_local_lookups_finish_within_budget() {
    for (declarations, uses) in [(8000, 2), (2, 8000)] {
        let mut literal = String::from("type Big<T> = {\n");
        let mut namespace = String::new();
        for index in 0..declarations {
            writeln!(literal, "m{index}: number;").unwrap();
            writeln!(
                namespace,
                "namespace Local {{ export type X{index} = number; }}"
            )
            .unwrap();
        }
        literal.push_str("last: T };\n");
        let last = declarations - 1;
        for index in 0..uses {
            writeln!(literal, "declare const l{index}: Big<number>;").unwrap();
            writeln!(namespace, "declare const n{index}: Local.X{last};").unwrap();
        }
        for (path, source) in [("literal.ts", literal), ("namespace.ts", namespace)] {
            let (hits, work) = run_with_work(registry(), path, source.as_bytes());
            assert!(hits.is_empty(), "{path}: {} findings", hits.len());
            assert_work(work, source.len(), 27, path);
        }
    }
}

#[test]
fn repeated_computed_keys_share_folded_prefixes() {
    let mut text = String::from("const b0 = '';\n");
    for i in 1..=14 {
        let prev = i - 1;
        writeln!(text, "const b{i} = b{prev} + b{prev};").unwrap();
    }
    for primed in [true, false] {
        let mut text = text.clone();
        if primed {
            text.push_str("consume({ [b14]: 0 });\n");
        }
        for i in 0..10_000 {
            writeln!(text, "consume({{ [b14 + 'shape{i}']: 0 }});").unwrap();
        }
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert_eq!(hits.len(), 10_000);
        assert!(hits.iter().enumerate().all(|(i, hit)| {
            hit.rule == "ts/no-shape-in-symbol-names" && hit.line == i + 16 + usize::from(primed)
        }));
        assert_work(
            work,
            text.len(),
            26,
            if primed { "primed" } else { "unprimed" },
        );
    }
}

#[test]
fn repeated_nonempty_key_prefixes_match_primed_cost() {
    let mut declarations = String::from("const b0 = 'x';\n");
    for i in 1..=14 {
        let prev = i - 1;
        writeln!(declarations, "const b{i} = b{prev} + b{prev};").unwrap();
    }
    declarations.push_str("const parts = { text: b14 };\n");
    for (label, prime, expression) in [
        ("left prefix", "b14", "b14 + '-shapeINDEX'"),
        ("right prefix", "b14", "'shape-INDEX-' + b14"),
        (
            "projected prefix",
            "parts.text",
            "parts.text + '-shapeINDEX'",
        ),
    ] {
        let [primed, unprimed] = [true, false].map(|primed| {
            let mut text = declarations.clone();
            if primed {
                writeln!(text, "consume({{ [{prime}]: 0 }});").unwrap();
            }
            let first = text.lines().count() + 1;
            for i in 0..2_000 {
                let key = expression.replace("INDEX", &i.to_string());
                writeln!(text, "consume({{ [{key}]: 0 }});").unwrap();
            }
            let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
            assert_eq!(hits.len(), 2_000, "{label}");
            assert!(hits.iter().enumerate().all(|(i, hit)| {
                hit.rule == "ts/no-shape-in-symbol-names" && hit.line == first + i
            }));
            work
        });
        assert_growth(primed, unprimed, 3, label);
    }
}

#[test]
fn repeated_dynamic_key_prefixes_share_failed_folds() {
    for (label, seed, join) in [
        ("left prefix", "getPrefix()", "PREV + ''"),
        ("right prefix", "getPrefix()", "'' + PREV"),
        ("projected prefix", "parts['text']", "PREV + ''"),
    ] {
        let [small, large] = [500, 2_000].map(|n| {
            let mut text = format!("const parts = {{ text: getPrefix() }};\nconst b0 = {seed};\n");
            for i in 1..=n {
                let prev = format!("b{}", i - 1);
                writeln!(text, "const b{i} = {};", join.replace("PREV", &prev)).unwrap();
            }
            for i in 0..n {
                writeln!(text, "consume({{ [b{n} + 'shape{i}']: 0 }});").unwrap();
            }
            writeln!(text, "consume({{ ['sha' + 'pe']: 0 }});").unwrap();
            let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
            assert_eq!(hits.len(), 1, "{label}");
            assert_eq!(hits[0].rule, "ts/no-shape-in-symbol-names");
            assert_eq!(hits[0].line, n * 2 + 3);
            work
        });
        assert_growth(small, large, 8, label);
    }
}

#[test]
fn repeated_cyclic_key_prefixes_share_failed_folds() {
    for (label, declarations) in [
        ("cyclic aliases", "const a = b + 'x'; const b = a + 'y';"),
        (
            "cycle on the right",
            "const a = 'x' + b; const b = 'y' + a;",
        ),
        (
            "cycle through a member",
            "const p = { lead: b }; const a = p.lead + 'x'; const b = a + 'y';",
        ),
    ] {
        let mut text = format!("{declarations}\n");
        for i in 0..5_000 {
            writeln!(text, "consume({{ [a + 'shape' + '{i}']: 0 }});").unwrap();
        }
        text.push_str("consume({ ['sha' + 'pe']: 0 });\n");
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert_eq!(hits.len(), 1, "{label}");
        assert_eq!(hits[0].rule, "ts/no-shape-in-symbol-names");
        assert_eq!(hits[0].line, 5_002);
        assert_work(work, text.len(), 28, label);
    }
}

#[test]
fn cyclic_shared_additions_share_failed_folds() {
    for (label, cycle, dynamic, join) in [
        (
            "left prefix",
            "const b0=b0+'';",
            "const b0=getPrefix();",
            "PREV+''",
        ),
        (
            "right prefix",
            "const b0=''+b0;",
            "const b0=getPrefix();",
            "''+PREV",
        ),
        (
            "projected prefix",
            "const p={key:b0}; const b0=p.key+'';",
            "const p={key:getPrefix()}; const b0=p.key+'';",
            "PREV+''",
        ),
    ] {
        let [fast, cyclic] = [dynamic, cycle].map(|seed| {
            let mut text = format!("{seed}\n");
            for i in 1..=4_000 {
                let prev = format!("b{}", i - 1);
                writeln!(text, "const b{i}={};", join.replace("PREV", &prev)).unwrap();
            }
            for _ in 0..4_000 {
                writeln!(text, "consume({{[b4000+'shape']:0}});").unwrap();
            }
            text.push_str("consume({['sha'+'pe']:0});\n");
            let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
            assert_eq!(hits.len(), 1, "{label}");
            assert_eq!(
                (hits[0].rule, hits[0].line),
                ("ts/no-shape-in-symbol-names", 8_002)
            );
            work
        });
        assert_growth(fast, cyclic, 8, label);
    }
}

#[test]
fn repeated_member_reads_do_not_rescan_other_effects() {
    for (label, effect) in [
        ("owner arguments", "consume(box);"),
        ("unrelated writes", "box.other=1;"),
        ("aliased bracket writes", "alias[field]=1;"),
    ] {
        let [small, large] = [500, 4_000].map(|n| {
            let mut text =
                String::from("const box={key:'shape'}; const alias=box; const field='other';\n");
            for _ in 0..n {
                writeln!(text, "{effect}").unwrap();
            }
            for _ in 0..n {
                text.push_str("consume({[box.key]:0});\n");
            }
            text.push_str("box.key='shape';\nconsume({[box.key]:0});\n");
            let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
            assert!(hits.is_empty(), "{label}");
            assert_work(work, text.len(), 39, label);
            work
        });
        assert_growth(small, large, 8, label);
    }
}

#[test]
fn long_computed_key_chains_share_fold_results() {
    for (label, seed, leaf, write) in [
        (
            "S53",
            "o.k + ''",
            "''".to_owned(),
            "o[k4000] = 'changed';\n",
        ),
        ("S54", "'shape'", format!("'{}'", "x".repeat(1_000)), ""),
    ] {
        let mut text =
            format!("const o = {{ k: 'shape' }}; const leaf = {leaf};\nconst k0 = {seed};\n");
        for i in 1..=4_000 {
            writeln!(text, "const k{i} = k{} + leaf;", i - 1).unwrap();
        }
        if !write.is_empty() {
            text.push_str("while (more()) {\n");
            text.push_str(write);
        }
        for _ in 0..4_000 {
            text.push_str("consume({ [k4000]: 0 });\n");
        }
        if !write.is_empty() {
            text.push_str("}\n");
        }
        text.push_str("consume({ ['sha' + 'pe']: 0 });\n");
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert_eq!(hits.len(), 1, "{label}");
        assert_eq!(
            (hits[0].rule, hits[0].line),
            ("ts/no-shape-in-symbol-names", text.lines().count()),
            "{label}"
        );
        assert_work(work, text.len(), 28, label);
    }
}

#[test]
fn long_typeof_paths_scale_linearly() {
    let [small, large] = [2_000, 16_000].map(|n| {
        let text = format!(
            "const a = {{}};\ntype T = typeof a{};\n",
            ".shape".repeat(n)
        );
        let (hits, work) = run_with_work(registry(), "query.ts", text.as_bytes());
        assert!(
            hits.iter().all(|h| h.rule != "ts/no-shape-in-symbol-names"),
            "{n}"
        );
        work
    });
    assert_growth(small, large, 16, "typeof paths");
}

#[test]
fn repeated_long_computed_keys_scale_linearly() {
    let [small, large] = [2_000, 12_000].map(|n| {
        let mut text = format!("const prefix = '{}';\n", "x".repeat(n));
        text.push_str(&"consume({ [prefix + '-id']: 0 });\n".repeat(n));
        text.push_str("consume({ [prefix + 's' + 'Ha' + 'Pe']: 0 });\n");
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].rule, hits[0].line),
            ("ts/no-shape-in-symbol-names", n + 2)
        );
        work
    });
    assert_growth(small, large, 8, "long computed keys");
}

#[test]
fn repeated_const_reads_in_nested_blocks_scale_linearly() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let [small, large] = [1_500, 10_000].map(|n| {
                let mut text = format!(
                    "{}const prefix = 'x'; const consume = () => {{}};\n{}{}\n",
                    "{".repeat(n),
                    "consume({ [prefix]: 0 });\n".repeat(n),
                    "}".repeat(n)
                );
                text.push_str("consume({ ['sha' + 'pe']: 0 });\n");
                let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
                assert_eq!(hits.len(), 1);
                assert_eq!(
                    (hits[0].rule, hits[0].line),
                    ("ts/no-shape-in-symbol-names", n + 3)
                );
                work
            });
            assert_growth(small, large, 8, "nested const reads");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn repeated_long_concatenated_selectors_stay_unknown() {
    let [small, large] = [4_000, 16_000].map(|n| {
        let prefix = "x".repeat(n);
        let mut text =
            format!("const prefix = '{prefix}';\nconst names = {{ '{prefix}-': 'userShape' }};\n");
        text.push_str(&"consume({ [names[prefix + '-']]: 0 });\n".repeat(n));
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert!(hits.is_empty(), "{n}: {} findings", hits.len());
        work
    });
    assert_growth(small, large, 6, "concatenated selectors");
}

#[test]
fn repeated_long_literal_selectors_scale_linearly() {
    let [small, large] = [8_000, 32_000].map(|n| {
        let key = "x".repeat(n);
        let mut text = format!("const key = '{key}'; const names = {{ '{key}': 'id' }};\n");
        text.push_str(&"consume({ [names[key]]: 0 });\n".repeat(n));
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert!(hits.is_empty(), "{n}: {} findings", hits.len());
        work
    });
    assert_growth(small, large, 6, "literal selectors");
}

#[test]
fn varying_concatenated_selectors_scale_linearly() {
    let [small, large] = [500, 2_000].map(|n| {
        let key = "x".repeat(2 * (n + 1));
        let mut text = format!("const names = {{ '{key}': 'id' }};\nconst p0 = 'x';\n");
        for i in 1..=n {
            writeln!(text, "const p{i} = p{} + 'x';", i - 1).unwrap();
        }
        for i in 1..=n {
            writeln!(text, "consume({{ [names[p{i} + p{n}]]: 0 }});").unwrap();
        }
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert!(hits.is_empty(), "{n}: {} findings", hits.len());
        work
    });
    assert_growth(small, large, 8, "varying selectors");
}

#[test]
fn nested_assignment_targets_scale_linearly_across_owners() {
    let [small, large] = [2_000, 12_000].map(|n| {
        let mut text = String::from("const defaults = { key: 'userShape' };\n");
        for i in 0..n {
            writeln!(text, "const o{i} = {{ key: 'userShape' }};").unwrap();
        }
        text.push_str("([o0.key = defaults.key, ");
        for i in 1..n {
            write!(text, "[o{i}.key, ").unwrap();
        }
        text.push_str("[]");
        text.push_str(&"]".repeat(n));
        text.push_str(" = input);\n");
        for i in 0..n {
            writeln!(text, "consume({{ [o{i}.key]: 0 }});").unwrap();
        }
        text.push_str("consume({ [defaults.key]: 0 });\n");
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].rule, hits[0].line),
            ("ts/no-shape-in-symbol-names", 2 * n + 3)
        );
        work
    });
    assert_growth(small, large, 8, "nested assignment targets");
}

#[test]
fn repeated_const_reads_with_an_unrelated_with_scale_linearly() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let [small, large] = [2_500, 10_000].map(|n| {
                // Local bindings keep Oxc from walking outward to resolve each read.
                let text = format!(
                    "with (o) {{ f(); }}\n{}const prefix = 'userShape'; const consume = () => {{}};\n{}{}",
                    "{".repeat(n),
                    "consume({ [prefix]: 0 });\n".repeat(n),
                    "}".repeat(n)
                );
                let (hits, work) = run_with_work(registry(), "computed.js", text.as_bytes());
                assert_eq!(hits.len(), n);
                assert!(hits.iter().all(|h| h.rule == "ts/no-shape-in-symbol-names"));
                work
            });
            assert_growth(small, large, 6, "const reads with an unrelated with");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn declaration_lookups_share_the_mutation_index() {
    let [small, large] = [250, 1_000].map(|n| {
        let text =
            "{ let value; consume(value); const key = 'userShape'; consume({ [key]: 0 }); }\n"
                .repeat(n);
        let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
        assert_eq!(hits.len(), n);
        assert!(
            hits.iter()
                .all(|hit| hit.rule == "ts/no-shape-in-symbol-names")
        );
        work
    });
    assert_growth(small, large, 8, "mutation index");
}

#[test]
fn plain_writes_under_deep_nesting_scale_linearly() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            for with in ["", "with (scope) {}\n"] {
                let [small, large] = [1_500, 10_000].map(|n| {
                    let text = format!(
                        "{with}const key = 'userShape';\nconst result = {{ [key]: 0 }};\nlet x;\n{}{}{};\n",
                        "(".repeat(n),
                        vec!["x = 1"; n].join(",\n"),
                        ")".repeat(n)
                    );
                    let (hits, work) = run_with_work(registry(), "writes.js", text.as_bytes());
                    assert_eq!(hits.len(), 1);
                    assert_eq!(
                        (hits[0].rule, hits[0].line),
                        ("ts/no-shape-in-symbol-names", 2 + usize::from(!with.is_empty()))
                    );
                    work
                });
                assert_growth(small, large, 8, "deep writes");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn early_empty_reads_in_nested_callbacks_scale_linearly() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let rule = registry()
                .iter()
                .find(|r| r.id == "ts/no-conditional-empty-object-spread")
                .unwrap();
            let [small, large] = [1_000, 4_000].map(|n| {
                let source = format!(
                    "{}0{}; const empty = {{}};",
                    "wrap(() => ({ ...(flag ? empty : payload) }), ".repeat(n),
                    ")".repeat(n)
                );
                let (findings, work) =
                    run_with_work(std::slice::from_ref(rule), "spread.ts", source.as_bytes());
                assert!(findings.is_empty());
                work
            });
            assert_growth(small, large, 6, "early reads in nested callbacks");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn function_call_member_chains_scale_linearly() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let rule = registry()
                .iter()
                .find(|r| r.id == "ts/no-conditional-empty-object-spread")
                .unwrap();
            let [small, large] = [500, 2_000].map(|n| {
                let source = format!(
                    "(() => ({{ ...(c ? empty : x) }})){}(); const empty = {{}};",
                    ".call".repeat(n)
                );
                let (hits, work) =
                    run_with_work(std::slice::from_ref(rule), "spread.ts", source.as_bytes());
                assert_eq!(hits.len(), 0);
                work
            });
            assert_growth(small, large, 6, "call member chains");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn wrapped_const_selectors_scale_linearly() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let [small, large] = [2_000, 8_000].map(|n| {
                let text = format!(
                    "const key = {}'label'{}; const names = {{ label: 'userShape' }};\n{}",
                    "(".repeat(n),
                    ")".repeat(n),
                    "consume({ [names[key]]: 0 });\n".repeat(n),
                );
                let (hits, work) = run_with_work(registry(), "computed.ts", text.as_bytes());
                assert_eq!(hits.len(), n);
                assert!(hits.iter().all(|h| h.rule == "ts/no-shape-in-symbol-names"));
                work
            });
            assert_growth(small, large, 6, "wrapped const selectors");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn reducer_resolution_scales_with_repeated_reads() {
    let rule = registry()
        .iter()
        .find(|rule| rule.id == "ts/no-reduce-accumulator-copy")
        .unwrap();
    for shape in ["aliases", "callbacks", "fresh targets"] {
        let [small, large] = [600, 2400].map(|n| {
            let (source, expected) = match shape {
                "aliases" => {
                    let mut text = String::from("function fold(acc) { const alias0 = acc;");
                    for i in 1..=n {
                        writeln!(text, "const alias{i} = alias{};", i - 1).unwrap();
                    }
                    for i in 0..n {
                        writeln!(text, "const copy{i} = [...alias{n}];").unwrap();
                    }
                    text.push_str("return acc; } [1].reduce(fold, []);");
                    (text, n)
                }
                "callbacks" => {
                    let mut text = String::from("function fold(acc) { const copy = [...acc];");
                    for _ in 0..n {
                        text.push_str("acc = other;");
                    }
                    text.push_str("return copy; }");
                    for _ in 0..n {
                        text.push_str("[1].reduce(fold, []);");
                    }
                    (text, 1)
                }
                _ => (
                    "[1].reduce(acc => { var next = {}; return Object.assign(next, acc); }, {});"
                        .repeat(n),
                    n,
                ),
            };
            let (hits, work) =
                run_with_work(std::slice::from_ref(rule), "rows.ts", source.as_bytes());
            assert_eq!(hits.len(), expected, "{shape} at {n}");
            work
        });
        assert_growth(small, large, 6, shape);
    }
}

#[test]
fn resolves_large_loader_inputs() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(time_loader_inputs)
        .unwrap()
        .join()
        .unwrap();
}

fn time_loader_inputs() {
    let config =
        blackhole_check::config::parse(b"[boundaries]\nbanned-imports=['restricted']").unwrap();
    let names: Vec<_> = (0..20_000).map(|i| format!("v{i}")).collect();
    let cases = [
        ("unknown member chain", format!("x{};", ".a".repeat(512))),
        (
            "object bindings",
            format!("const {{{}}}=x; {};", names.join(","), names.join(";")),
        ),
        (
            "call chain 2500",
            format!("require{}();", ".call".repeat(2_500)),
        ),
        (
            "call chain 5000",
            format!("require{}();", ".call".repeat(5_000)),
        ),
        (
            "call chain 10000",
            format!("require{}();", ".call".repeat(10_000)),
        ),
        (
            "apply chain 10000",
            format!("require{}();", ".apply".repeat(10_000)),
        ),
        (
            "bind chain 10000",
            format!("require{}();", ".bind(null)".repeat(10_000)),
        ),
        (
            "nested calls 3000",
            format!("{}0{};", "f(require,".repeat(3_000), ")".repeat(3_000)),
        ),
        (
            "repeated loader",
            format!("const r=require; {}", "r('x');".repeat(20_000)),
        ),
    ];
    for (root, n) in [
        ("require", 2500),
        ("require", 5000),
        ("require", 10000),
        ("arguments", 30000),
    ] {
        let source = format!(
            "{}0{};",
            "consume(".repeat(n),
            format!(",{root})").repeat(n)
        );
        let start = Instant::now();
        let hits = blackhole_check::engine::run_configured(
            registry(),
            config.engine_settings(),
            [("large.cjs", source.as_bytes())],
        );
        let elapsed = start.elapsed();
        assert_eq!(
            hits.iter()
                .filter(|f| f.rule == "boundary/banned-imports")
                .count(),
            n
        );
        eprintln!("nested {root} mentions {n} {elapsed:?}");
        assert!(elapsed < Duration::from_secs(8));
    }
    for (label, mut source) in cases {
        source.push_str("\nrequire('restricted');");
        let start = Instant::now();
        let hits = blackhole_check::engine::run_configured(
            registry(),
            config.engine_settings(),
            [("large.ts", source.as_bytes())],
        );
        let elapsed = start.elapsed();
        assert!(
            hits.iter().any(|f| f.rule == "boundary/banned-imports"
                && f.line == 2
                && f.note.as_deref() == Some("restricted")),
            "{label}"
        );
        eprintln!("{label} {elapsed:?}");
        assert!(elapsed < Duration::from_secs(8), "{label} took {elapsed:?}");
    }
}
