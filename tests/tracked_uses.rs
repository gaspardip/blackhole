use blackhole_check::engine::{registry, run};

#[test]
fn mock_arguments() {
    for (source, expected) in [
        ("consume(vi.mock)", Some((12, Some("mock")))),
        ("consume(vi)", Some((9, Some("vi")))),
        ("consume(vi.fn)", Some((12, Some("fn")))),
        ("consume(custom)", None),
        ("Reflect.apply(vi.mock, vi, [\"client\"])", Some((18, None))),
    ] {
        let findings = run(registry(), [("case.test.ts", source.as_bytes())]);
        let actual: Vec<_> = findings
            .iter()
            .map(|f| (f.rule, f.line, f.col, f.note.clone()))
            .collect();
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(col, name)| {
                (
                    "ts/no-test-doubles",
                    1,
                    col,
                    name.map(|n| format!("cannot read how `{n}` is used here; call it by name")),
                )
            })
            .collect();
        assert_eq!(actual, expected, "{source}");
    }
}

#[test]
fn unbound_suite_argument() {
    for name in ["context", "describe"] {
        let source = format!("new Auth({name}, env)");
        let found = run(registry(), [("case.test.ts", source.as_bytes())]);
        let expected = format!("cannot read how `{name}` is used here; call it by name");
        assert_eq!(
            found
                .iter()
                .map(|h| (h.rule, h.line, h.col, h.note.as_deref()))
                .collect::<Vec<_>>(),
            [("ts/no-skipped-tests", 1, 10, Some(expected.as_str()))]
        );
    }
}

#[test]
fn callback_from_spread_belongs_to_runner() {
    let source = "const args=['x', async () => { await new Promise(r => setTimeout(r, 10)); expect(read()).toBe(1); }]; test(...args);";
    let findings = run(registry(), [("case.test.ts", source.as_bytes())]);
    assert_eq!(
        findings.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["ts/no-fixed-sleep"]
    );
}

#[test]
fn global_jest_argument() {
    let dir = "rules/ts/no-test-aware-code/reject";
    for (file, col) in [
        ("global-global.ts", 12),
        ("global-globalthis.ts", 16),
        ("global-window.ts", 12),
    ] {
        let source = std::fs::read(std::path::Path::new(dir).join(file)).unwrap();
        let found = run(registry(), [(file, source.as_slice())]);
        assert_eq!(
            found
                .iter()
                .map(|f| (f.rule, f.line, f.col))
                .collect::<Vec<_>>(),
            [
                ("ts/no-test-aware-code", 1, 5),
                ("ts/no-test-doubles", 1, col)
            ],
            "{file}"
        );
    }
    let found = run(registry(), [("runtime.ts", b"use(jest);".as_slice())]);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.line, f.col))
            .collect::<Vec<_>>(),
        [("ts/no-test-doubles", 1, 5)]
    );
}

#[test]
fn bound_intrinsic_reports_only_the_skip() {
    for source in [
        "const run = Reflect.apply.bind(undefined);\nrun(test.skip, undefined, ['x', () => assert(read())]);",
        "const run = Function.prototype.call.bind(test.skip, undefined);\nrun('x', () => assert(read()));",
    ] {
        let found = run(registry(), [("case.test.ts", source.as_bytes())]);
        assert_eq!(
            found
                .iter()
                .map(|f| (f.rule, f.line, f.col, f.note.as_deref()))
                .collect::<Vec<_>>(),
            [("ts/no-skipped-tests", 2, 1, None)],
            "{source}"
        );
    }
}

#[test]
fn opaque_runner_options_are_reported() {
    for (source, expected) in [
        (
            "import {test} from 'node:test'; test('x', readOptions(), () => assert(read()));",
            1,
        ),
        (
            "import {test} from 'vitest'; test('x', readOptions(), () => assert(read()));",
            1,
        ),
        ("test('x', readOptions(), () => assert(read()));", 1),
        (
            "import {test} from 'node:test'; test('x', readOptions());",
            1,
        ),
        (
            "import {test} from 'node:test'; test('x', () => assert(read()));",
            0,
        ),
        (
            "import {test} from 'node:test'; function body() { assert(read()); } test('x', body);",
            0,
        ),
        (
            "import {test} from 'node:test'; function register(body: () => void) { test('x', body); }",
            0,
        ),
        (
            "import {test} from 'node:test'; import {body} from './body'; test('x', body);",
            1,
        ),
        (
            "import {test} from '@jest/globals'; import {body} from './body'; test('x', body);",
            0,
        ),
        (
            "function test(name, options, body) {} test('x', readOptions(), () => assert(read()));",
            0,
        ),
    ] {
        let found = run(registry(), [("case.test.ts", source.as_bytes())]);
        assert_eq!(
            found
                .iter()
                .filter(|f| f.rule == "ts/no-skipped-tests")
                .count(),
            expected,
            "{source}"
        );
    }
}
