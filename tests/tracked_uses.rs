use blackhole_check::engine::{registry, run};

const SKIP: &str = "ts/no-skipped-tests";
const FLAKE: &str = "ts/no-flake-masking";

#[test]
fn test_double_usage_findings() {
    let doubles = "ts/no-test-doubles";
    let reflect = "ts/no-reflect-apply";
    let opaque = |col, name| {
        (
            doubles,
            1,
            col,
            Some(format!(
                "cannot read how `{name}` is used here; call it by name"
            )),
        )
    };
    for (file, source, expected) in [
        ("case.test.ts", "consume(vi.mock)", vec![opaque(12, "mock")]),
        ("case.test.ts", "consume(vi)", vec![opaque(9, "vi")]),
        ("case.test.ts", "consume(vi.fn)", vec![opaque(12, "fn")]),
        ("case.test.ts", "consume(custom)", vec![]),
        (
            "case.test.ts",
            "Reflect.apply(vi.mock, vi, [\"client\"])",
            vec![(reflect, 1, 1, None), (doubles, 1, 18, None)],
        ),
        (
            "src/native-instrumentation.ts",
            r#"Reflect.apply(Reflect.set, Reflect, [navigator, "gpu", gpu]);"#,
            vec![(reflect, 1, 1, None)],
        ),
        (
            "spy.fixture.tsx",
            r#"Reflect.apply(vi.spyOn, vi, [client, "send"]);"#,
            vec![(reflect, 1, 1, None)],
        ),
        (
            "spy-only-mock.fixture.tsx",
            r#"Reflect.apply(vi.mock, vi, ["client", { spy: true }]);"#,
            vec![(reflect, 1, 1, None)],
        ),
        (
            "recording-wrapper.fixture.tsx",
            "const original = client.send; client.send = function (...args) { calls.push(args); return Reflect.apply(original, this, args); }; client.send = original;",
            vec![(reflect, 1, 91, None)],
        ),
        (
            "forwarding-wrapper.fixture.tsx",
            "const original = client.send; client.send = function (...args) { return Reflect.apply(original, this, args); }; client.send = original;",
            vec![(reflect, 1, 73, None)],
        ),
    ] {
        let findings = run(registry(), [(file, source.as_bytes())]);
        let actual: Vec<_> = findings
            .iter()
            .map(|f| (f.rule, f.line, f.col, f.note.clone()))
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
        "const run=Reflect.apply.bind(undefined,test.skip,undefined);\nrun(['sample',()=>assert(read())]);",
        "const run = Reflect.apply.bind(undefined);\nrun(test.skip, undefined, ['x', () => assert(read())]);",
        "const run = Function.prototype.call.bind(test.skip, undefined);\nrun('x', () => assert(read()));",
    ] {
        let found = run(registry(), [("case.test.ts", source.as_bytes())]);
        assert_eq!(
            found
                .iter()
                .map(|f| (f.rule, f.line, f.col, f.note.as_deref()))
                .collect::<Vec<_>>(),
            [(SKIP, 2, 1, None)],
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

#[test]
fn conditional_arguments_inspect_both_result_arms() {
    let check = |source: &str| {
        run(registry(), [("case.test.ts", source.as_bytes())])
            .into_iter()
            .map(|f| (f.rule, f.line, f.note))
            .collect::<Vec<_>>()
    };
    let source = "consume(flag ? test.skip : ordinary);\n\
                  consume(flag ? ordinary : test.only);\n\
                  consume(flag ? vi.mock : ordinary);\n\
                  consume(flag ? ordinary : vi.fn);\n\
                  consume(test.skip ? ordinary : other);\n\
                  { const vi = helpers; consume(flag ? vi.mock : ordinary); }";
    assert_eq!(
        check(source),
        [
            (
                "ts/no-skipped-tests",
                1,
                Some("cannot read how `skip` is used here; call it by name".into())
            ),
            (
                "ts/no-skipped-tests",
                2,
                Some("cannot read how `only` is used here; call it by name".into())
            ),
            (
                "ts/no-test-doubles",
                3,
                Some("cannot read how `mock` is used here; call it by name".into())
            ),
            (
                "ts/no-test-doubles",
                4,
                Some("cannot read how `fn` is used here; call it by name".into())
            ),
        ]
    );
}

#[test]
fn default_runner_api_escapes_and_reassigned_skips() {
    let source = "import nodeTest from 'node:test';\n\
                  import playwrightTest from '@playwright/test';\n\
                  consume(nodeTest.skip);\n\
                  [playwrightTest.slow];\n\
                  let selected = fallback;\n\
                  if (flag) selected = nodeTest.skip;\n\
                  selected('example', () => assert(read()));\n\
                  nodeTest.skip('control', () => assert(read()));\n\
                  playwrightTest.slow();";
    let found = run(registry(), [("case.test.ts", source.as_bytes())]);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.line, f.col, f.note.as_deref()))
            .collect::<Vec<_>>(),
        [
            (
                SKIP,
                3,
                18,
                Some("cannot read how `skip` is used here; call it by name")
            ),
            (
                FLAKE,
                4,
                17,
                Some("cannot read how `slow` is used here; call it by name")
            ),
            (SKIP, 7, 1, None),
            (SKIP, 8, 10, None),
            (FLAKE, 9, 1, None),
        ]
    );
}

#[test]
fn flake_masking_config_api_escapes() {
    let source = "import {test} from '@playwright/test';\n\
                  consume(vi.setConfig);\n\
                  [test.describe.configure];\n\
                  vi.setConfig({retry: 2});\n\
                  test.describe.configure({retries: 2});\n\
                  { const vi = custom; consume(vi.setConfig); }";
    let found = run(registry(), [("case.test.ts", source.as_bytes())]);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.line, f.col, f.note.as_deref()))
            .collect::<Vec<_>>(),
        [
            (
                FLAKE,
                2,
                12,
                Some("cannot read how `setConfig` is used here; call it by name")
            ),
            (
                FLAKE,
                3,
                16,
                Some("cannot read how `configure` is used here; call it by name")
            ),
            (FLAKE, 4, 15, None),
            (FLAKE, 5, 26, None),
        ]
    );
}

#[test]
fn runner_option_slots_follow_the_api() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "template title",
            "import {test} from 'node:test'; test(`sample ${name}`, () => assert(read()));",
            &[],
        ),
        (
            "concatenated Node title",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; test('renders' + ' sample', () => equal(2+2, 4));",
            &[],
        ),
        (
            "Node title from a string-returning function",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; function title(): string { return 'declared sample'; } test(title(), () => equal(2+2, 4));",
            &[],
        ),
        (
            "Node title factory shadowed by an options factory",
            "import {test} from 'node:test'; function title(): string { return 'outer'; } function register(title: () => object) { test(title(), () => assert(read())); }",
            &[FLAKE, SKIP],
        ),
        (
            "global known options first",
            "test({skip: true}, () => assert(read()));",
            &[SKIP],
        ),
        (
            "bound callback",
            "import {test} from 'node:test'; function check() { assert(read()); } test('sample', check.bind(null));",
            &[],
        ),
        (
            "bound imported callback",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; test('sample', equal.bind(null, 2+2, 4));",
            &[],
        ),
        (
            "shadowed bound imported callback",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; function register(equal) { test('sample', equal.bind(null, 2+2, 4)); }",
            &[SKIP],
        ),
        (
            "bound opaque options from an unrelated module",
            "import {test} from 'node:test'; import {equal} from './options'; test('sample', equal.bind(null), () => assert(read()));",
            &[FLAKE, SKIP],
        ),
        (
            "options first",
            "import {test} from 'node:test'; test(readOptions(), () => assert(read()));",
            &[FLAKE, SKIP],
        ),
        (
            "named spread",
            "import {test} from 'node:test'; test('sample', ...readArgs());",
            &[SKIP],
        ),
        (
            "options first timeout",
            "import {test} from 'node:test'; test({timeout: 100}, () => assert(read()));",
            &[FLAKE],
        ),
        (
            "named timeout",
            "import {test} from 'node:test'; test('sample', {timeout: 100}, () => assert(read()));",
            &[FLAKE],
        ),
        (
            "options first skip disabled",
            "import {test} from 'node:test'; test({skip: false}, () => assert(read()));",
            &[],
        ),
        (
            "Playwright factory details",
            "import {test, type TestDetails} from '@playwright/test'; function details(): TestDetails { return {tag: '@fast'}; } const browserTest = test.extend({}); browserTest('sample', details(), () => assert(read()));",
            &[],
        ),
        (
            "global legacy timeout",
            "function check() { assert(read()); } test('sample', check, 100);",
            &[FLAKE],
        ),
    ];
    assert_option_slots(cases);
}

fn assert_option_slots(cases: &[(&str, &str, &[&str])]) {
    let actual: Vec<_> = cases
        .iter()
        .map(|(case, source, _)| {
            let mut hits: Vec<_> = run(registry(), [("case.test.ts", source.as_bytes())])
                .into_iter()
                .filter_map(|f| [FLAKE, SKIP].contains(&f.rule).then_some(f.rule))
                .collect();
            hits.sort_unstable();
            (*case, hits)
        })
        .collect();
    let expected: Vec<_> = cases
        .iter()
        .map(|(case, _, hits)| (*case, hits.to_vec()))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn node_titles_trust_only_stable_string_signatures() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "Node title method replaced after its declaration",
            "import {test} from 'node:test'; const provider = { title(): string { return 'sample'; } }; Object.assign(provider, { title: () => ({skip: true}) }); test(provider.title(), () => assert(read()));",
            &[FLAKE, SKIP],
        ),
        (
            "Node title from a parameter typed to return a string",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; function register(title: () => string) { test(title(), () => equal(2+2, 4)); }",
            &[],
        ),
        (
            "Node title from a variable typed to return a string",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; const title: () => string = makeTitle; test(title(), () => equal(2+2, 4));",
            &[],
        ),
        (
            "Node title from a reassigned parameter typed to return a string",
            "import {test} from 'node:test'; function register(title: () => string, other) { title = other; test(title(), () => assert(read())); }",
            &[FLAKE, SKIP],
        ),
        (
            "Node title from a destructured callable variable",
            "import {test} from 'node:test'; const provider = Object.assign(() => 'sample', { toString: () => ({skip: true}) }); const {toString: title}: () => string = provider; test(title(), () => { throw new Error('callback ran'); });",
            &[FLAKE, SKIP],
        ),
        (
            "Node title from a destructured callable parameter",
            "import {test} from 'node:test'; function register({toString: title}: () => string) { test(title(), () => { throw new Error('callback ran'); }); } register(Object.assign(() => 'sample', { toString: () => ({skip: true}) }));",
            &[FLAKE, SKIP],
        ),
        (
            "Node title from an annotated replaced member",
            "import {test} from 'node:test'; const provider = { title: () => 'sample' }; const replacement = { title: () => ({skip: true}) }; const title: () => string = Object.assign(provider, replacement).title; test(title(), () => { throw new Error('callback ran'); });",
            &[FLAKE, SKIP],
        ),
        (
            "Node title from an annotated alias of a member",
            "import {test} from 'node:test'; const provider = { title: () => 'sample' }; const member = Object.assign(provider, { title: () => ({skip: true}) }).title; const title: () => string = member; test(title(), () => { throw new Error('callback ran'); });",
            &[FLAKE, SKIP],
        ),
        (
            "Node title from a stable alias of an annotated factory",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; const provider: () => string = () => 'stable alias'; const alias = provider; const title = alias; test(title(), () => equal(2+2, 4));",
            &[],
        ),
        (
            "Node title from a parenthesized callable type",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; const title: (() => string) = () => 'parenthesized callable'; test(title(), () => equal(2+2, 4));",
            &[],
        ),
        (
            "Node title from a parenthesized return type",
            "import {test} from 'node:test'; import {equal} from 'node:assert/strict'; const title: () => (string) = () => 'parenthesized return'; test(title(), () => equal(2+2, 4));",
            &[],
        ),
    ];
    assert_option_slots(cases);
}

fn assert_review_rules(path: &str, source: &str, expected: &[&str]) {
    let hits = run(registry(), [(path, source.as_bytes())]);
    assert_eq!(
        hits.iter().map(|hit| hit.rule).collect::<Vec<_>>(),
        expected,
        "{source}"
    );
}

#[test]
fn review_r10_node_string_titles() {
    for source in [
        r#"import {test} from "node:test"; import assert from "node:assert/strict"; function register(name: string) { test(name, () => assert.equal(read(), 1)); }"#,
        r#"import {test} from "node:test"; import assert from "node:assert/strict"; const cb = () => assert.equal(read(), 1); function register(name: string) { test(String(name), cb); }"#,
    ] {
        assert_review_rules("case.test.ts", source, &[]);
    }
}

#[test]
fn review_r11_application_retry_property() {
    assert_review_rules(
        "src/build.ts",
        r#"export const commands = { test: { retry: "manual" } }"#,
        &[],
    );
}

#[test]
fn review_r13_application_implementation_method() {
    assert_review_rules(
        "case.test.ts",
        r#"class Renderer { withImplementation(mode: string) { render(mode); } } new Renderer().withImplementation("fast")"#,
        &[],
    );
}

#[test]
fn review_r14_forwarded_route_response_metadata() {
    assert_review_rules(
        "case.test.ts",
        r#"import type {Route} from "@playwright/test"; async function forward(route: Route) { const response = await route.fetch(); await route.fulfill({ response, contentType: "application/json" }); }"#,
        &[],
    );
}

#[test]
fn review_r01_bare_expect_without_matcher() {
    assert_review_rules(
        "case.test.ts",
        r#"test("x", () => { expect(read()); })"#,
        &["ts/no-assertionless-tests"],
    );
}

#[test]
fn review_r03_node_context_skip_and_todo() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test} from "node:test"; import assert from "node:assert/strict"; test("x", t => { t.skip(); assert.equal(read(), 1); }); test("todo", t => { t.todo(); assert.equal(read(), 1); });"#,
        &["ts/no-skipped-tests", "ts/no-skipped-tests"],
    );
}

#[test]
fn review_r04_conditional_call_targets() {
    assert_review_rules(
        "case.test.ts",
        r#"const run = disabled ? test.skip : test; run("x", () => assert(read())); const create = fake ? vi.fn : factory; create(() => 1);"#,
        &["ts/no-skipped-tests", "ts/no-test-doubles"],
    );
}

#[test]
fn review_r05_fake_timers_do_not_leak_between_tests() {
    assert_review_rules(
        "case.test.ts",
        r#"test("timer control", () => { vi.useFakeTimers(); vi.useRealTimers(); assert(read()); }); test("real sleep", async () => { await new Promise(r => setTimeout(r, 10)); assert(read()); });"#,
        &["ts/no-fixed-sleep"],
    );
}

#[test]
fn review_r07_unreadable_config_spread() {
    assert_review_rules(
        "case.test.ts",
        "vi.setConfig(...configArgs())",
        &["ts/no-flake-masking"],
    );
}

#[test]
fn review_r08_playwright_page_fixture_route() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test} from "@playwright/test"; test("x", async ({page}) => { await page.route("**/api", route => route.fulfill({ json: { success: true } })); assert(read()); });"#,
        &["ts/no-test-doubles"],
    );
}
