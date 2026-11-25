use blackhole_check::engine::{registry, run};
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
    for shape in [
        "object-spreads",
        "aliased-spreads",
        "array-spreads",
        "overwrites",
        "imported",
        "letalias",
        "fnhelper",
        "callchain",
        "chaincalls",
        "chain",
    ] {
        let text = source(shape, 10_000);
        let start = Instant::now();
        let hits = run(registry(), [("large.test.ts", text.as_bytes())]);
        assert!(hits.is_empty(), "{shape}");
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "{shape} took {:?}",
            start.elapsed()
        );
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
    let start = Instant::now();
    let hits = run(registry(), [("case.test.ts", text.as_bytes())]);
    assert_eq!(
        hits.iter().map(|hit| hit.rule).collect::<Vec<_>>(),
        Vec::<&str>::new()
    );
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "shared SDK option spreads took {:?}",
        start.elapsed()
    );
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
        let start = Instant::now();
        let hits = run(registry(), [("defaults.ts", source.as_bytes())]);
        assert!(start.elapsed() < Duration::from_secs(3));
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
            let start = Instant::now();
            let hits = run(registry(), [(path, source.as_bytes())]);
            assert!(hits.is_empty(), "{path}: {} findings", hits.len());
            assert!(start.elapsed() < Duration::from_secs(3), "{path}");
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
        let start = Instant::now();
        let hits = run(registry(), [("computed.ts", text.as_bytes())]);
        let elapsed = start.elapsed();
        eprintln!("repeated computed keys (primed: {primed}) took {elapsed:?}");
        assert_eq!(hits.len(), 10_000);
        assert!(hits.iter().enumerate().all(|(i, hit)| {
            hit.rule == "ts/no-shape-in-symbol-names" && hit.line == i + 16 + usize::from(primed)
        }));
        assert!(elapsed < Duration::from_secs(8), "primed: {primed}");
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
    let mut slow = Vec::new();
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
            let start = Instant::now();
            let hits = run(registry(), [("computed.ts", text.as_bytes())]);
            let elapsed = start.elapsed();
            assert_eq!(hits.len(), 2_000, "{label}");
            assert!(hits.iter().enumerate().all(|(i, hit)| {
                hit.rule == "ts/no-shape-in-symbol-names" && hit.line == first + i
            }));
            elapsed
        });
        eprintln!("{label}: primed {primed:?}, unprimed {unprimed:?}");
        if unprimed >= primed * 3 + Duration::from_millis(150) {
            slow.push(label);
        }
    }
    assert!(slow.is_empty(), "slow without priming: {slow:?}");
}

#[test]
fn repeated_dynamic_key_prefixes_share_failed_folds() {
    let mut slow = Vec::new();
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
            let start = Instant::now();
            let hits = run(registry(), [("computed.ts", text.as_bytes())]);
            let elapsed = start.elapsed();
            assert_eq!(hits.len(), 1, "{label}");
            assert_eq!(hits[0].rule, "ts/no-shape-in-symbol-names");
            assert_eq!(hits[0].line, n * 2 + 3);
            elapsed
        });
        eprintln!("{label}: 500 keys {small:?}, 2000 keys {large:?}");
        if large >= small * 8 + Duration::from_millis(15) {
            slow.push(label);
        }
    }
    assert!(slow.is_empty(), "repeated dynamic prefix walks: {slow:?}");
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
        let start = Instant::now();
        let hits = run(registry(), [("computed.ts", text.as_bytes())]);
        let elapsed = start.elapsed();
        eprintln!("{label}: 5000 keys took {elapsed:?}");
        assert_eq!(hits.len(), 1, "{label}");
        assert_eq!(hits[0].rule, "ts/no-shape-in-symbol-names");
        assert_eq!(hits[0].line, 5_002);
        assert!(elapsed < Duration::from_secs(8), "{label}");
    }
}

#[test]
fn cyclic_shared_additions_share_failed_folds() {
    let mut slow = Vec::new();
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
            let start = Instant::now();
            let hits = run(registry(), [("computed.ts", text.as_bytes())]);
            let elapsed = start.elapsed();
            assert_eq!(hits.len(), 1, "{label}");
            assert_eq!(
                (hits[0].rule, hits[0].line),
                ("ts/no-shape-in-symbol-names", 8_002)
            );
            elapsed
        });
        eprintln!("{label}: dynamic {fast:?}, cyclic {cyclic:?}");
        if cyclic >= fast * 8 + Duration::from_millis(100) {
            slow.push(label);
        }
    }
    assert!(slow.is_empty(), "repeated cyclic prefix walks: {slow:?}");
}

#[test]
fn repeated_member_reads_do_not_rescan_other_effects() {
    let mut slow = Vec::new();
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
            let start = Instant::now();
            let hits = run(registry(), [("computed.ts", text.as_bytes())]);
            let elapsed = start.elapsed();
            assert!(hits.is_empty(), "{label}");
            elapsed
        });
        eprintln!("{label}: 500 effects and reads {small:?}, 4000 {large:?}");
        if large >= small * 8 + Duration::from_millis(100) || large >= Duration::from_secs(2) {
            slow.push(label);
        }
    }
    assert!(slow.is_empty(), "repeated member effect scans: {slow:?}");
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
        let start = Instant::now();
        let hits = run(registry(), [("computed.ts", text.as_bytes())]);
        let elapsed = start.elapsed();
        eprintln!("{label}: 4000 additions and reads took {elapsed:?}");
        assert_eq!(hits.len(), 1, "{label}");
        assert_eq!(
            (hits[0].rule, hits[0].line),
            ("ts/no-shape-in-symbol-names", text.lines().count()),
            "{label}"
        );
        assert!(elapsed < Duration::from_secs(2), "{label}");
    }
}

#[test]
fn long_typeof_paths_scale_linearly() {
    let [small, large] = [2_000, 16_000].map(|n| {
        let text = format!(
            "const a = {{}};\ntype T = typeof a{};\n",
            ".shape".repeat(n)
        );
        let start = Instant::now();
        let hits = run(registry(), [("query.ts", text.as_bytes())]);
        let elapsed = start.elapsed();
        assert!(
            hits.iter().all(|h| h.rule != "ts/no-shape-in-symbol-names"),
            "{n}"
        );
        elapsed
    });
    eprintln!("typeof paths: 2000 segments {small:?}, 16000 {large:?}");
    assert!(
        large < small * 16 + Duration::from_millis(100),
        "{small:?} {large:?}"
    );
}

#[test]
fn repeated_long_computed_keys_scale_linearly() {
    let [small, large] = [2_000, 12_000].map(|n| {
        let mut text = format!("const prefix = '{}';\n", "x".repeat(n));
        text.push_str(&"consume({ [prefix + '-id']: 0 });\n".repeat(n));
        text.push_str("consume({ [prefix + 's' + 'Ha' + 'Pe']: 0 });\n");
        let start = Instant::now();
        let hits = run(registry(), [("computed.ts", text.as_bytes())]);
        let elapsed = start.elapsed();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].rule, hits[0].line),
            ("ts/no-shape-in-symbol-names", n + 2)
        );
        elapsed
    });
    eprintln!("long computed keys: 2000 {small:?}, 12000 {large:?}");
    assert!(
        large < small * 8 + Duration::from_millis(25),
        "{small:?} {large:?}"
    );
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
                let start = Instant::now();
                let hits = run(registry(), [("computed.ts", text.as_bytes())]);
                let elapsed = start.elapsed();
                assert_eq!(hits.len(), 1);
                assert_eq!(
                    (hits[0].rule, hits[0].line),
                    ("ts/no-shape-in-symbol-names", n + 3)
                );
                elapsed
            });
            eprintln!("nested const reads: 1500 {small:?}, 10000 {large:?}");
            assert!(
                large < small * 8 + Duration::from_millis(25),
                "{small:?} {large:?}"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn repeated_long_concatenated_selectors_stay_unknown() {
    let [small, large] = [2_000, 16_000].map(|n| {
        let prefix = "x".repeat(n);
        let mut text =
            format!("const prefix = '{prefix}';\nconst names = {{ '{prefix}-': 'userShape' }};\n");
        text.push_str(&"consume({ [names[prefix + '-']]: 0 });\n".repeat(n));
        let start = Instant::now();
        let hits = run(registry(), [("computed.ts", text.as_bytes())]);
        let elapsed = start.elapsed();
        assert!(hits.is_empty(), "{n}: {} findings", hits.len());
        elapsed
    });
    eprintln!("computed selectors: 2000 {small:?}, 16000 {large:?}");
    assert!(
        large < small * 8 + Duration::from_millis(25),
        "{small:?} {large:?}"
    );
}

#[test]
fn repeated_long_literal_selectors_scale_linearly() {
    let [small, large] = [8_000, 32_000].map(|n| {
        let key = "x".repeat(n);
        let mut text = format!("const key = '{key}'; const names = {{ '{key}': 'id' }};\n");
        text.push_str(&"consume({ [names[key]]: 0 });\n".repeat(n));
        let start = Instant::now();
        let hits = run(registry(), [("computed.ts", text.as_bytes())]);
        let elapsed = start.elapsed();
        assert!(hits.is_empty(), "{n}: {} findings", hits.len());
        elapsed
    });
    eprintln!("literal selectors: 8000 {small:?}, 32000 {large:?}");
    assert!(
        large < small * 6 + Duration::from_millis(25),
        "{small:?} {large:?}"
    );
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
        let start = Instant::now();
        let hits = run(registry(), [("computed.ts", text.as_bytes())]);
        let elapsed = start.elapsed();
        assert!(hits.is_empty(), "{n}: {} findings", hits.len());
        elapsed
    });
    eprintln!("varying concatenated selectors: 500 {small:?}, 2000 {large:?}");
    assert!(
        large < small * 8 + Duration::from_millis(25),
        "{small:?} {large:?}"
    );
}

#[test]
fn repeated_const_reads_with_an_unrelated_with_scale_linearly() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let [small, large] = [1_250, 10_000].map(|n| {
                // Local bindings keep Oxc from walking outward to resolve each read.
                let text = format!(
                    "with (o) {{ f(); }}\n{}const prefix = 'userShape'; const consume = () => {{}};\n{}{}",
                    "{".repeat(n),
                    "consume({ [prefix]: 0 });\n".repeat(n),
                    "}".repeat(n)
                );
                let start = Instant::now();
                let hits = run(registry(), [("computed.js", text.as_bytes())]);
                let elapsed = start.elapsed();
                assert_eq!(hits.len(), n);
                assert!(hits.iter().all(|h| h.rule == "ts/no-shape-in-symbol-names"));
                elapsed
            });
            eprintln!("nested const reads with an unrelated with: 1250 {small:?}, 10000 {large:?}");
            assert!(
                large < small * 8 + Duration::from_millis(25),
                "{small:?} {large:?}"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
