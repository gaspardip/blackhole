use blackhole_check::engine::{registry, run};

struct Row {
    rule: &'static str,
    file: &'static str,
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
    for (label, callee) in [
        ("dot", p.to_owned()),
        ("paren", format!("({p})")),
        ("comma", format!("(0,{p})")),
        ("as", format!("({p} as any)")),
        ("satisfies", format!("({p} satisfies any)")),
        ("nonnull", format!("({p})!")),
    ] {
        add(label, String::new(), format!("{callee}({a})"));
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
            ("wrapped key", format!("{o}[(\"{k}\")]")),
            ("template", format!("{o}[\u{60}{k}\u{60}]")),
            ("nonnull within chain", format!("({o}?.{k}!)")),
            ("optional member", format!("{o}?.{k}")),
            ("wrapped object", format!("({o}).{k}")),
            ("as object", format!("({o} as any).{k}")),
            ("satisfies object", format!("({o} satisfies any).{k}")),
            ("nonnull object", format!("{o}!.{k}")),
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
        for root in ["window", "globalThis"] {
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
    if !row.callback.is_empty() {
        add(
            "wrapped callback",
            String::new(),
            format!(
                "{p}({})",
                a.replace(row.callback, &format!("({})", row.callback))
            ),
        );
    }
}
const ROWS: &[Row] = &[
    Row {
        file: "case.test.ts",
        rule: "no-test-doubles",
        path: "Object.assign",
        args: "window,{fetch:fake}",
        module: "",
        prelude: "",
        context: "CALL",
        options: "",
        callback: "",
    },
    Row {
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
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
        file: "case.test.ts",
    },
];

#[test]
fn spellings() {
    let mut missing = Vec::new();
    let mut count = 0;
    for row in ROWS {
        for (label, source) in cases(row) {
            count += 1;
            if !run(registry(), [(row.file, source.as_bytes())])
                .iter()
                .any(|h| h.rule.strip_prefix("ts/") == Some(row.rule))
            {
                missing.push(format!("{} {label}: {source}", row.rule));
            }
        }
    }
    let chained = "import * as m from 'msw/node'; (m?.setupServer()!).listen()";
    assert!(
        run(registry(), [("chain.test.ts", chained.as_bytes())])
            .iter()
            .any(|h| h.rule == "ts/strict-msw"),
        "{chained}"
    );
    eprintln!("{} call spellings", count + 1);
    assert!(
        missing.is_empty(),
        "{} of {count} missing:\n{}",
        missing.len(),
        missing.join("\n")
    );
}
