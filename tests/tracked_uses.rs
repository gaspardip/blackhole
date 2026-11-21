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
fn review_r02_constant_expect_subjects() {
    assert_review_rules(
        "case.test.ts",
        r#"test("aliases", () => { const actual = 1; expect(actual).toBe(1); }); test("soft", () => { expect.soft(1).toBe(1); });"#,
        &["ts/no-assertionless-tests", "ts/no-assertionless-tests"],
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
fn review_r06_unhandled_request_handlers_must_fail() {
    let cases = [
        ("onUnhandledRequest() {}", true),
        (
            "onUnhandledRequest(request, print) { print.warning(); }",
            true,
        ),
        ("onUnhandledRequest() { console.warn('unhandled'); }", true),
        ("onUnhandledRequest: () => console.log('unhandled')", true),
        (
            "onUnhandledRequest() { console.info('unhandled'); console.warn('unhandled'); }",
            true,
        ),
        (r#"onUnhandledRequest: "error""#, false),
        (
            "onUnhandledRequest(request, print) { print.error(); }",
            false,
        ),
        (
            "onUnhandledRequest(request) { unhandled.push(request.url); }",
            false,
        ),
        (
            "onUnhandledRequest(request, print) { print.warning(); unhandled.push(request.url); }",
            false,
        ),
    ];
    let actual: Vec<_> = cases.iter().map(|(handler, _)| {
        let source = format!(
            r#"import {{setupServer}} from "msw/node"; const network = setupServer(); network.listen({{ {handler} }});"#
        );
        let rules: Vec<_> = run(registry(), [("case.test.ts", source.as_bytes())])
            .into_iter().map(|finding| finding.rule).collect();
        (*handler, rules)
    }).collect();
    let expected: Vec<_> = cases
        .iter()
        .map(|(handler, report)| {
            (
                *handler,
                if *report {
                    vec!["ts/strict-msw"]
                } else {
                    vec![]
                },
            )
        })
        .collect();
    assert_eq!(actual, expected);
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

#[test]
fn review_r09_default_process_import() {
    assert_review_rules(
        "src/runtime.ts",
        r#"import process from "process"; read(process.env.VITEST);"#,
        &["ts/no-test-aware-code"],
    );
}

#[test]
fn review_r12_negated_poll_length() {
    assert_review_rules(
        "case.test.ts",
        "expect.poll(() => requests).not.toHaveLength(0)",
        &["ts/no-unsignaled-poll"],
    );
}

#[test]
fn review_r15_playwright_test_timeout() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test} from "@playwright/test"; test("x", () => { test.setTimeout(60_000); assert(read()); });"#,
        &["ts/no-flake-masking"],
    );
}

#[test]
fn review_r16_reassigned_runner_escape() {
    assert_review_rules(
        "case.test.ts",
        r#"import test from "@playwright/test"; let run = registered; if (omitCase) run = test.skip; handoff(run)"#,
        &["ts/no-skipped-tests"],
    );
}

#[test]
fn review_r17_node_before_after_sleep() {
    assert_review_rules(
        "case.test.ts",
        r#"import {before, after} from "node:test"; before(async () => { await new Promise(r => setTimeout(r, 10)); }); after(async () => { await new Promise(r => setTimeout(r, 10)); });"#,
        &["ts/no-fixed-sleep", "ts/no-fixed-sleep"],
    );
}

#[test]
fn review_r18_destructured_native_write() {
    assert_review_rules(
        "case.test.ts",
        "[globalThis.fetch] = [fakeFetch]",
        &["ts/no-test-doubles"],
    );
}

#[test]
fn review_r19_named_assertionless_callback() {
    assert_review_rules(
        "case.test.ts",
        r#"function exerciseOnly() { run(); } test("x", exerciseOnly)"#,
        &["ts/no-assertionless-tests"],
    );
}

#[test]
fn round2_r21_fake_timers_in_hooks() {
    assert_review_rules(
        "case.test.ts",
        r#"beforeEach(() => { vi.useFakeTimers(); }); afterEach(() => { vi.useRealTimers(); }); test("timer", async () => { const elapsed = new Promise(r => setTimeout(r, 10)); vi.advanceTimersByTime(10); await elapsed; assert(read()); });"#,
        &[],
    );
    assert_review_rules(
        "case.test.ts",
        r#"describe("fake", () => { beforeAll(() => { vi.useFakeTimers(); }); test("fake", async () => { await scheduler.wait(10); assert(read()); }); }); describe("real", () => { test("real", async () => { await scheduler.wait(10); assert(read()); }); });"#,
        &["ts/no-fixed-sleep", "ts/no-fixed-sleep"],
    );
    assert_review_rules(
        "case.test.ts",
        r#"vi.useFakeTimers(); test("fake", async () => { await scheduler.wait(10); assert(read()); });"#,
        &["ts/no-fixed-sleep"],
    );
}

#[test]
fn round2_r22_constant_threshold_dynamic_result() {
    assert_review_rules(
        "case.test.ts",
        r#"test("rows", async () => { const minimum = 2; expect(minimum).toBeLessThan(await countRows()); });"#,
        &[],
    );
}

#[test]
fn round2_r23_playwright_hook_timeout() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test, expect} from "@playwright/test"; test.beforeAll(async () => { test.setTimeout(60_000); await seedDatabase(); expect(await isDatabaseReady()).toBe(true); });"#,
        &[],
    );
}

#[test]
fn round2_r24_escape_before_assignment() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test} from "@playwright/test"; let callback = save; handoff(callback); callback = test; callback("x", () => assert(read()));"#,
        &[],
    );
    for source in [
        r#"import {test} from "@playwright/test"; let callback = save; for (const item of items) { handoff(callback); callback = test; }"#,
        r#"import {test} from "@playwright/test"; let callback = save; install(); handoff(callback); function install() { callback = test; }"#,
    ] {
        assert_review_rules("case.test.ts", source, &[SKIP]);
    }
}

#[test]
fn round2_r25_named_callback_local_helper() {
    assert_review_rules(
        "case.test.ts",
        r#"function checkDatabase() { expect(read()).toBe("ready"); } function exercise() { checkDatabase(); } test("reads database", exercise);"#,
        &[],
    );
    assert_review_rules(
        "case.test.ts",
        r#"function checkDatabase() { expect(read()).toBe("ready"); } test("reads database", () => checkDatabase());"#,
        &[],
    );
}

#[test]
fn round2_r14_conditional_forwarded_route_metadata() {
    assert_review_rules(
        "case.test.ts",
        r#"import type {Route} from "@playwright/test"; async function forward(route: Route) { const response = await route.fetch(); await route.fulfill(useJson ? {response, contentType: "application/json"} : {response, contentType: "text/plain"}); }"#,
        &[],
    );
}

#[test]
fn round2_r13_class_method_this() {
    assert_review_rules(
        "case.test.ts",
        r#"class Renderer { withImplementation(mode: string) { render(mode); } setup() { this.withImplementation("fast"); } }"#,
        &[],
    );
}

#[test]
fn round2_r10_node_string_title_alias() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test} from "node:test"; const callback = () => assert(read()); function register(name: string) { const title = name; test(title, callback); }"#,
        &[],
    );
}

#[test]
fn round2_r20_config_callback_return() {
    assert_review_rules(
        "vitest.config.ts",
        r#"import {defineConfig} from "vitest/config"; export default defineConfig(() => ({test: {retry: 2}}));"#,
        &["ts/no-flake-masking"],
    );
}

#[test]
fn round2_r20_config_block_return() {
    assert_review_rules(
        "vitest.config.ts",
        r#"import {defineConfig} from "vitest/config"; export default defineConfig(() => { if (process.env.CI) return {test: {retry: 2}}; return {}; });"#,
        &[FLAKE],
    );
}

#[test]
fn round2_r03_named_node_context() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test, type TestContext} from "node:test"; test("x", exercise); function exercise(t: TestContext) { t.skip(); }"#,
        &["ts/no-assertionless-tests", "ts/no-skipped-tests"],
    );
}

#[test]
fn round2_r08_named_playwright_fixture() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test, type Page} from "@playwright/test"; test("x", exercise); async function exercise({page}: {page: Page}) { await page.route("**/api", r => r.fulfill({json: {ok: true}})); }"#,
        &["ts/no-assertionless-tests", "ts/no-test-doubles"],
    );
}

#[test]
fn round2_r15_named_playwright_test_timeout() {
    assert_review_rules(
        "case.test.ts",
        r#"import {test} from "@playwright/test"; test("x", exercise); function exercise() { test.setTimeout(60_000); assert(read()); }"#,
        &["ts/no-flake-masking"],
    );
}

#[test]
fn round2_r17_named_node_hook_sleep() {
    assert_review_rules(
        "case.test.ts",
        r#"import {before} from "node:test"; before(setup); async function setup() { await new Promise(r => setTimeout(r, 10)); }"#,
        &["ts/no-fixed-sleep"],
    );
}

#[test]
fn round2_r06_returned_console_and_destructured_warning() {
    assert_review_rules(
        "case.test.ts",
        r#"import {setupServer} from "msw/node"; const network = setupServer(); network.listen({ onUnhandledRequest(req) { return console.warn(req.url); } }); network.listen({ onUnhandledRequest(req) { console.error(req.url); } }); network.listen({ onUnhandledRequest: (_, {warning}) => warning() }); network.listen({ onUnhandledRequest: (_, {error}) => error() }); network.listen({ onUnhandledRequest() { throw Error("unhandled"); } });"#,
        &["ts/strict-msw", "ts/strict-msw", "ts/strict-msw"],
    );
}

#[test]
fn round2_r18_native_shorthand_target() {
    assert_review_rules(
        "case.test.ts",
        "({fetch} = {fetch: fakeFetch})",
        &["ts/no-test-doubles"],
    );
}

#[test]
fn round3_r26_playwright_hook_page_fixture() {
    let source = r#"import {test, expect} from "@playwright/test"; test.beforeEach(async ({page}) => { await page.route("**/api", route => route.fulfill({json: {data: []}})); await page.goto("/rows"); expect(await page.title()).toBe("rows"); })"#;
    for hook in ["beforeEach", "afterEach", "beforeAll", "afterAll"] {
        let source = source.replace("beforeEach", hook);
        let col = source.find("route.fulfill").unwrap() + 1;
        let hits = run(registry(), [("case.test.ts", source.as_bytes())]);
        assert_eq!(
            hits.iter()
                .map(|hit| (hit.rule, hit.line, hit.col))
                .collect::<Vec<_>>(),
            [("ts/no-test-doubles", 1, col)],
            "{hook}"
        );
    }
    assert_review_rules(
        "case.test.ts",
        r#"import {beforeEach} from "node:test"; beforeEach(t => { t.skip(); });"#,
        &[],
    );
}

#[test]
fn round3_r28_after_each_restores_once_timers() {
    assert_review_rules(
        "case.test.ts",
        r#"beforeAll(() => { vi.useFakeTimers(); }); afterEach(() => { vi.useRealTimers(); }); test("first", () => assert(read())); test("second", async () => { await new Promise(r => setTimeout(r, 10)); assert(read()); });"#,
        &["ts/no-fixed-sleep"],
    );
    for source in [
        r#"vi.useFakeTimers(); afterEach(() => { vi.useRealTimers(); }); test("first", () => assert(read())); test("second", async () => { await scheduler.wait(10); assert(read()); });"#,
        r#"describe("outer", () => { afterEach(() => { vi.useRealTimers(); }); describe("nested", () => { beforeAll(() => { vi.useFakeTimers(); }); test("first", () => assert(read())); test("second", async () => { await scheduler.wait(10); assert(read()); }); }); });"#,
    ] {
        assert_review_rules("case.test.ts", source, &["ts/no-fixed-sleep"]);
    }
}

#[test]
fn round3_r20_default_config_function() {
    let source = "export default function config() { return {test: {retry: 2}}; }";
    assert_review_rules("vitest.config.ts", source, &[FLAKE]);
    assert_review_rules("src/build.ts", source, &[]);
}

#[test]
fn round3_r20_default_config_arrow() {
    assert_review_rules(
        "vitest.config.ts",
        "export default () => ({test: {retry: 2}})",
        &["ts/no-flake-masking"],
    );
}

#[test]
fn round3_r06_warning_then_bare_return() {
    assert_review_rules(
        "case.test.ts",
        r#"import {setupServer} from "msw/node"; const network = setupServer(); network.listen({ onUnhandledRequest(req) { console.warn(req.url); return; } });"#,
        &["ts/strict-msw"],
    );
}

#[test]
fn round4_r30_first_test_precedes_timer_restore() {
    assert_review_rules(
        "case.test.ts",
        r#"beforeAll(() => { vi.useFakeTimers(); }); afterEach(() => { vi.useRealTimers(); }); test("timer", async () => { const elapsed = new Promise(r => setTimeout(r, 10)); vi.advanceTimersByTime(10); await elapsed; assert(read()); });"#,
        &["ts/no-fixed-sleep"],
    );
}

#[test]
fn round4_r31_before_all_precedes_timer_restore() {
    assert_review_rules(
        "case.test.ts",
        r#"vi.useFakeTimers(); beforeAll(async () => { const elapsed = new Promise(r => setTimeout(r, 10)); vi.advanceTimersByTime(10); await elapsed; }); afterEach(() => vi.useRealTimers()); test("timer", () => assert(read()));"#,
        &["ts/no-fixed-sleep"],
    );
    assert_review_rules(
        "case.test.ts",
        r#"vi.useFakeTimers(); afterEach(() => vi.useRealTimers()); test("timer", () => assert(read())); beforeAll(async () => { const elapsed = new Promise(r => setTimeout(r, 10)); vi.advanceTimersByTime(10); await elapsed; });"#,
        &["ts/no-fixed-sleep"],
    );
}

#[test]
fn round4_timer_hooks_report_with_file_timers() {
    for hook in ["beforeEach", "afterEach"] {
        let first = format!(
            r#"vi.useFakeTimers(); {hook}(async () => {{ await scheduler.wait(10); }}); afterEach(() => vi.useRealTimers()); test("first", () => assert(read()));"#
        );
        assert_review_rules("case.test.ts", &first, &["ts/no-fixed-sleep"]);
        let second = format!(r#"{first} test("second", () => assert(read()));"#);
        assert_review_rules("case.test.ts", &second, &["ts/no-fixed-sleep"]);
    }
    assert_review_rules(
        "case.test.ts",
        r#"vi.useFakeTimers(); afterAll(async () => { await scheduler.wait(10); }); afterEach(() => vi.useRealTimers()); test("timer", () => assert(read()));"#,
        &["ts/no-fixed-sleep"],
    );
}

#[test]
fn round5_r32_after_each_sleep_after_timer_restore() {
    assert_review_rules(
        "case.test.ts",
        r#"import { afterEach, expect, test, vi } from "vitest";

vi.useFakeTimers();
afterEach(async () => {
  await new Promise(resolve => setTimeout(resolve, 10));
});
afterEach(() => { vi.useRealTimers(); });
test("reads the clock", () => expect(Date.now()).toBeGreaterThan(0));
"#,
        &["ts/no-fixed-sleep"],
    );
}

#[test]
fn round5_r33_file_timers_do_not_exempt_named_or_inline_suites() {
    let source = r#"import { afterEach, describe, expect, test, vi } from "vitest";

function smokeTests() {
  afterEach(() => { vi.useRealTimers(); });
  test("reads the clock", () => expect(Date.now()).toBeGreaterThan(0));
}

vi.useFakeTimers();
test("advances the clock", async () => {
  const start = Date.now();
  const elapsed = new Promise(resolve => setTimeout(resolve, 10));
  vi.advanceTimersByTime(10);
  await elapsed;
  expect(Date.now()).toBe(start + 10);
});
describe("smoke tests", smokeTests);
"#;
    assert_review_rules("case.test.ts", source, &["ts/no-fixed-sleep"]);
    let inline = r#"describe("smoke tests", () => {
  afterEach(() => { vi.useRealTimers(); });
  test("reads the clock", () => expect(Date.now()).toBeGreaterThan(0));
});
"#;
    let inline = source
        .replace(
            r#"function smokeTests() {
  afterEach(() => { vi.useRealTimers(); });
  test("reads the clock", () => expect(Date.now()).toBeGreaterThan(0));
}

"#,
            "",
        )
        .replace(r#"describe("smoke tests", smokeTests);"#, inline);
    assert_review_rules("case.test.ts", &inline, &["ts/no-fixed-sleep"]);
}

#[test]
fn round5_before_each_timer_inheritance() {
    let source = r#"function cases() {
  test("nested", async () => { await scheduler.wait(10); assert(read()); });
}
describe("outer", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  describe("inner", cases);
});"#;
    assert_review_rules("case.test.ts", source, &[]);
    let with_restore = source.replace(
        "  test(",
        "  beforeEach(() => { vi.useRealTimers(); vi.useFakeTimers(); });\n  test(",
    );
    assert_review_rules("case.test.ts", &with_restore, &["ts/no-fixed-sleep"]);
    let with_local_restore =
        source.replace("await scheduler", "vi.useRealTimers(); await scheduler");
    assert_review_rules("case.test.ts", &with_local_restore, &["ts/no-fixed-sleep"]);
}

#[test]
fn round5_hooks_use_their_own_timers() {
    for (setup, expected) in [
        ("", &["ts/no-fixed-sleep"][..]),
        ("jest.useFakeTimers();", &[]),
    ] {
        let source = format!(
            r#"beforeEach(() => {{ vi.useFakeTimers(); }}); afterEach(async () => {{ {setup} await scheduler.wait(10); }}); test("x", () => assert(read()));"#
        );
        assert_review_rules("case.test.ts", &source, expected);
    }
}

fn assert_fixed_sleep_at(source: &str, line: usize, col: usize) {
    let hits = run(registry(), [("case.test.ts", source.as_bytes())]);
    assert_eq!(
        hits.iter()
            .map(|hit| (hit.rule, hit.line, hit.col))
            .collect::<Vec<_>>(),
        [("ts/no-fixed-sleep", line, col)],
        "{source}"
    );
}

#[test]
fn round6_r34_fake_timer_options_must_cover_timeouts() {
    assert_fixed_sleep_at(
        r#"import { afterEach, beforeEach, expect, jest, test } from "@jest/globals";

beforeEach(() => {
  jest.useFakeTimers({ now: 0, doNotFake: ["setTimeout"] });
});
afterEach(() => { jest.useRealTimers(); });

test("keeps the date fixed during a real delay", async () => {
  const start = Date.now();
  await new Promise(resolve => setTimeout(resolve, 10));
  expect(Date.now()).toBe(start);
});
"#,
        10,
        3,
    );
    assert_review_rules(
        "case.test.ts",
        r#"beforeEach(() => { vi.useFakeTimers({ now: 0 }); }); test("clock", async () => { await new Promise(resolve => setTimeout(resolve, 10)); assert(read()); });"#,
        &[],
    );
    for setup in [
        "const config = { now: 0 }; vi.useFakeTimers(config);",
        "vi.useFakeTimers(...[]);",
        "const install = vi.useFakeTimers.bind(vi, ...[]); install();",
        "jest.useFakeTimers('legacy');",
        "vi.useFakeTimers({ ...{} });",
        "vi.useFakeTimers({ toFake: ['Date'] });",
    ] {
        let source = format!(
            r#"beforeEach(() => {{ {setup} }}); test("clock", async () => {{ await scheduler.wait(10); assert(read()); }});"#
        );
        assert_review_rules("case.test.ts", &source, &["ts/no-fixed-sleep"]);
    }
}

#[test]
fn round6_r35_shared_cases_require_fake_timers_at_every_call() {
    let source = r#"import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

function clockCases() {
  test("fires a timeout", async () => {
    const elapsed = new Promise(resolve => setTimeout(() => resolve("done"), 10));
    if (vi.isFakeTimers()) vi.advanceTimersByTime(10);
    const result = await elapsed;
    expect(result).toBe("done");
  });
}

describe("real clock", () => { clockCases(); });
describe("fake clock", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });
  describe("shared cases", clockCases);
});
"#;
    assert_fixed_sleep_at(source, 7, 20);
    let fake_clock = source.replace(
        "clockCases();",
        "beforeEach(() => { vi.useFakeTimers(); }); clockCases();",
    );
    assert_review_rules("case.test.ts", &fake_clock, &[]);
    let opaque = source.replace("clockCases();", "handoff(clockCases);");
    assert_fixed_sleep_at(&opaque, 7, 20);
}

#[test]
fn round6_r36_fake_timers_must_precede_scheduling() {
    let source = r#"import { expect, jest, test } from "@jest/globals";

test("keeps the date fixed while a pending delay completes", async () => {
  const elapsed = new Promise(resolve => setTimeout(resolve, 10));
  jest.useFakeTimers({ now: 0 });
  try {
    await elapsed;
    expect(Date.now()).toBe(0);
  } finally {
    jest.useRealTimers();
  }
});
"#;
    assert_fixed_sleep_at(source, 7, 5);
    let helper = source.replace(
        "new Promise(resolve => setTimeout(resolve, 10))",
        "scheduler.wait(10)",
    );
    assert_fixed_sleep_at(&helper, 7, 5);
}
