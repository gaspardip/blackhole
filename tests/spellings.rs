use blackhole_check::engine::{registry, run};

const TEST_FILE: &str = "case.test.ts";
const WRAPPERS: &[(&str, &str)] = &[
    ("paren", "(VALUE)"),
    ("comma", "(0,VALUE)"),
    ("as", "((VALUE) as any)"),
    ("satisfies", "((VALUE) satisfies any)"),
    ("nonnull", "(VALUE)!"),
];

struct Row {
    rule: &'static str,
    path: &'static str,
    args: &'static str,
    module: &'static str,
    prelude: &'static str,
    context: &'static str,
    options: &'static str,
    callback: &'static str,
}

fn cases(row: &Row) -> Vec<(String, String)> {
    let p = row.path;
    let a = row.args;
    let mut cases = Vec::new();
    let mut add = |label: &str, decl: String, call: String| {
        cases.push((
            label.to_owned(),
            format!(
                "{} {decl} {}",
                row.prelude,
                row.context.replace("CALL", &call)
            ),
        ));
    };
    add("dot", String::new(), format!("{p}({a})"));
    for &(label, wrapper) in WRAPPERS {
        let callee = wrapper.replace("VALUE", p);
        add(label, String::new(), format!("{callee}({a})"));
        if let Some((owner, key)) = p.rsplit_once('.') {
            let object = wrapper.replace("VALUE", owner);
            let key_expr = wrapper.replace("VALUE", &format!("'{key}'"));
            add(
                &format!("{label} object"),
                String::new(),
                format!("{object}.{key}({a})"),
            );
            add(
                &format!("{label} key"),
                String::new(),
                format!("{owner}[{key_expr}]({a})"),
            );
        }
        for (slot, value) in [("options", row.options), ("callback", row.callback)] {
            if !value.is_empty() {
                let args = a.replace(value, &wrapper.replace("VALUE", value));
                add(
                    &format!("{label} {slot}"),
                    String::new(),
                    format!("{p}({args})"),
                );
            }
        }
    }
    add("optional call", String::new(), format!("{p}?.({a})"));
    add("whole call", String::new(), format!("({p}({a}))"));
    add(
        "method alias",
        format!("const method={p};"),
        format!("method({a})"),
    );
    let receiver = p.rsplit_once('.').map_or("undefined", |(owner, _)| owner);
    for (label, call) in [
        ("call", format!("{p}.call({receiver},{a})")),
        ("apply", format!("{p}.apply({receiver},[{a}])")),
        ("bind", format!("{p}.bind({receiver})({a})")),
        ("reflect", format!("Reflect.apply({p},{receiver},[{a}])")),
    ] {
        add(label, String::new(), call);
    }
    if let Some((o, k)) = p.rsplit_once('.') {
        for (label, callee) in [
            ("bracket", format!("{o}[\"{k}\"]")),
            ("template", format!("{o}[\u{60}{k}\u{60}]")),
            ("nonnull within chain", format!("({o}?.{k}!)")),
            ("optional member", format!("{o}?.{k}")),
        ] {
            add(label, String::new(), format!("{callee}({a})"));
        }
        if row.prelude.is_empty() && !o.contains('.') {
            add(
                "qualified shadowed root",
                format!("const {o}=custom;"),
                format!("globalThis['{o}'][`{k}`]({a})"),
            );
        }
        add(
            "object alias",
            format!("const obj={o};"),
            format!("obj.{k}({a})"),
        );
        add(
            "destructure",
            format!("const {{{k}: method}}={o};"),
            format!("method({a})"),
        );
    }
    if row.prelude.is_empty() {
        add(
            "global alias",
            "const g=globalThis;".into(),
            format!("g.{p}({a})"),
        );
        for root in ["window", "globalThis", "self"] {
            add(root, String::new(), format!("{root}.{p}({a})"));
        }
    }
    arguments_and_imports(row, &mut add);
    cases
}

fn arguments_and_imports(row: &Row, add: &mut impl FnMut(&str, String, String)) {
    let p = row.path;
    let a = row.args;
    if !row.module.is_empty() {
        let root = p.split('.').next().unwrap();
        let suffix = &p[root.len()..];
        let m = row.module;
        for (label, decl, name) in [
            (
                "named",
                format!("import {{{root}}} from '{m}';"),
                root.to_owned(),
            ),
            (
                "renamed",
                format!("import {{{root} as alias}} from '{m}';"),
                "alias".into(),
            ),
            (
                "namespace",
                format!("import * as lib from '{m}';"),
                format!("lib.{root}"),
            ),
        ] {
            add(label, decl, format!("{name}{suffix}({a})"));
        }
        if m == "node:test" {
            add(
                "default",
                format!("import alias from '{m}';"),
                format!("alias{suffix}({a})"),
            );
        }
    }
    if !row.options.is_empty() {
        for (label, v) in [("options alias", "opts"), ("spread", "{...opts}")] {
            add(
                label,
                format!("const opts={};", row.options),
                format!("{p}({})", a.replace(row.options, v)),
            );
        }
    }
}
const ROWS: &[Row] = &[
    Row {
        rule: "no-test-doubles",
        path: "Object.assign",
        args: "window,{fetch:fake}",
        module: "",
        prelude: "",
        context: "CALL",
        options: "{fetch:fake}",
        callback: "",
    },
    Row {
        rule: "no-test-doubles",
        path: "vi.mock",
        args: "'client'",
        module: "vitest",
        prelude: "",
        context: "CALL",
        options: "",
        callback: "",
    },
    Row {
        rule: "no-flake-masking",
        path: "vi.setConfig",
        args: "{retry:2}",
        module: "vitest",
        prelude: "",
        context: "CALL",
        options: "{retry:2}",
        callback: "",
    },
    Row {
        rule: "no-skipped-tests",
        path: "test.skip",
        args: "'x',f",
        module: "node:test",
        prelude: "",
        context: "CALL",
        options: "",
        callback: "f",
    },
    Row {
        rule: "no-fixed-sleep",
        path: "scheduler.wait",
        args: "1",
        module: "",
        prelude: "",
        context: "test('x',async()=>{await CALL;assert(work())})",
        options: "",
        callback: "",
    },
    Row {
        rule: "no-unsignaled-poll",
        path: "vi.waitFor",
        args: "()=>expect(calls).toHaveLength(1)",
        module: "vitest",
        prelude: "",
        context: "CALL",
        options: "",
        callback: "()=>expect(calls).toHaveLength(1)",
    },
    Row {
        rule: "no-assertionless-tests",
        path: "test",
        args: "'x',()=>{}",
        module: "vitest",
        prelude: "",
        context: "CALL",
        options: "",
        callback: "()=>{}",
    },
    Row {
        rule: "strict-msw",
        path: "server.listen",
        args: "",
        module: "",
        prelude: "import {setupServer} from 'msw/node'; const server=setupServer();",
        context: "CALL",
        options: "",
        callback: "",
    },
    Row {
        rule: "no-unsignaled-poll",
        path: "expect.poll",
        args: "()=>calls.length",
        context: "CALL",
        module: "vitest",
        prelude: "",
        options: "",
        callback: "()=>calls.length",
    },
    Row {
        rule: "no-fixed-sleep",
        path: "Bun.sleep",
        args: "1",
        context: "test('x',async()=>{await CALL;assert(work())})",
        module: "",
        prelude: "",
        options: "",
        callback: "",
    },
    Row {
        rule: "no-flake-masking",
        path: "test.slow",
        args: "",
        context: "CALL",
        module: "@playwright/test",
        prelude: "",
        options: "",
        callback: "",
    },
    Row {
        rule: "no-flake-masking",
        path: "test.describe.configure",
        args: "{retries:2}",
        context: "CALL",
        module: "@playwright/test",
        prelude: "",
        options: "{retries:2}",
        callback: "",
    },
    Row {
        rule: "no-flake-masking",
        path: "test",
        args: "'x',{retry:2},f",
        context: "CALL",
        module: "vitest",
        prelude: "",
        options: "{retry:2}",
        callback: "f",
    },
    Row {
        rule: "strict-msw",
        path: "worker.start",
        args: "{onUnhandledRequest:'bypass'}",
        context: "CALL",
        module: "",
        prelude: "import {setupWorker} from 'msw/browser';const worker=setupWorker();",
        options: "{onUnhandledRequest:'bypass'}",
        callback: "",
    },
    Row {
        rule: "no-fixed-sleep",
        path: "setTimeout",
        args: "r,100",
        module: "",
        prelude: "",
        context: "test('x',async()=>{await new Promise(r=>CALL);assert(ok())})",
        options: "",
        callback: "r",
    },
    Row {
        rule: "no-skipped-tests",
        path: "test",
        args: "'x',{skip:true},fn",
        module: "node:test",
        prelude: "",
        context: "CALL",
        options: "{skip:true}",
        callback: "fn",
    },
    Row {
        rule: "strict-msw",
        path: "(m?.setupServer()!).listen",
        args: "",
        module: "",
        prelude: "import * as m from 'msw/node';",
        context: "CALL",
        options: "",
        callback: "",
    },
];

const ENV_ROWS: &[(&str, &str)] = &[("import.meta.env", "MODE"), ("process.env", "NODE_ENV")];

fn environment_cases(owner: &str, key: &str) -> Vec<String> {
    let mut cases = vec![format!("const env={owner}; env.{key} === 'test';")];
    for &(_, wrapper) in WRAPPERS {
        let object = wrapper.replace("VALUE", owner);
        let member = wrapper.replace("VALUE", &format!("'{key}'"));
        let read = wrapper.replace("VALUE", &format!("{owner}.{key}"));
        cases.extend([
            format!("{object}.{key} === 'test'"),
            format!("{owner}[{member}] === 'test'"),
            format!("{read} === 'test'"),
        ]);
    }
    cases
}

#[test]
fn spellings() {
    let mut missing = Vec::new();
    let mut count = 0;
    for row in ROWS {
        for (label, source) in cases(row) {
            count += 1;
            if !run(registry(), [(TEST_FILE, source.as_bytes())])
                .iter()
                .any(|h| h.rule.strip_prefix("ts/") == Some(row.rule))
            {
                missing.push(format!("{} {label}: {source}", row.rule));
            }
        }
    }
    for &(owner, key) in ENV_ROWS {
        for source in environment_cases(owner, key) {
            count += 1;
            if !run(registry(), [("src/env.ts", source.as_bytes())])
                .iter()
                .any(|h| h.rule == "ts/no-test-aware-code")
            {
                missing.push(source);
            }
        }
    }
    eprintln!("{} rows, {count} spellings", ROWS.len() + ENV_ROWS.len());
    assert!(
        missing.is_empty(),
        "{} of {count} missing:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn tagged_calls_without_an_offence() {
    for (import, callee) in [("test", "test"), ("vi", "vi.waitFor")] {
        for call in [format!("{callee}`tag`"), format!("{callee}([\"tag\"])")] {
            let source = format!("import {{ {import} }} from 'vitest'; {call};");
            let findings = run(registry(), [(TEST_FILE, source.as_bytes())]);
            assert!(findings.is_empty(), "{source}");
        }
    }
}

#[test]
fn wrapped_and_aliased_constant_assertions() {
    let source = "const input = 1;\n\
                  test('checks', () => {\n\
                  assert((0, true));\n\
                  expect((0, true)).toBe(false);\n\
                  expect(input);\n\
                  assert((0, read()));\n\
                  expect((0, read())).toBe(true);\n\
                  expect(read()).toBe(input);\n\
                  });";
    let hits = run(registry(), [(TEST_FILE, source.as_bytes())]);
    assert_eq!(
        hits.iter().map(|f| (f.rule, f.line)).collect::<Vec<_>>(),
        [
            ("ts/no-assertionless-tests", 3),
            ("ts/no-assertionless-tests", 4),
            ("ts/no-assertionless-tests", 5)
        ]
    );
}

#[test]
fn reflected_tautology_points_to_the_matcher() {
    let source =
        "test('checks', () => {\nReflect.apply(expect(true).toBe, undefined, [false]);\n});";
    let hits = run(registry(), [(TEST_FILE, source.as_bytes())]);
    assert_eq!(
        hits.iter()
            .map(|f| (f.rule, f.line, f.col))
            .collect::<Vec<_>>(),
        [
            ("ts/no-reflect-apply", 2, 1),
            ("ts/no-assertionless-tests", 2, 28)
        ]
    );
}

#[test]
fn poll_and_recorder_aliases_match_direct_expressions() {
    let source = "expect.poll(() => calls).toHaveLength(1);\n\
                  const pending = expect.poll(() => calls);\n\
                  pending.toHaveLength(1);\n\
                  expect.poll(() => calls.current.length).toBe(1);\n\
                  const currentCalls = calls.current;\n\
                  expect.poll(() => currentCalls.length).toBe(1);\n\
                  const polledState = expect.poll(() => read());\n\
                  polledState.toHaveLength(1);\n\
                  const recorder = calls;\n\
                  expect.poll(() => recorder.length).toBe(1);";
    let hits = run(registry(), [(TEST_FILE, source.as_bytes())]);
    assert_eq!(
        hits.iter()
            .map(|f| (f.rule, f.line, f.col))
            .collect::<Vec<_>>(),
        [
            ("ts/no-unsignaled-poll", 1, 1),
            ("ts/no-unsignaled-poll", 2, 17),
            ("ts/no-unsignaled-poll", 10, 1)
        ]
    );
}
