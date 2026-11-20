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
