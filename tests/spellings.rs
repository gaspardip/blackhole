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
        let body = if row.context.contains("DECL") {
            row.context.replace("DECL", &decl)
        } else {
            format!("{decl} {}", row.context)
        };
        cases.push((
            label.to_owned(),
            format!("{} {}", row.prelude, body.replace("CALL", &call)),
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

fn check_spelling(
    missing: &mut Vec<String>,
    label: &str,
    rule: &str,
    source: &str,
    offset: Option<usize>,
) {
    check_file_spelling(missing, label, rule, (TEST_FILE, source), offset);
}

fn check_file_spelling(
    missing: &mut Vec<String>,
    label: &str,
    rule: &str,
    (path, source): (&str, &str),
    offset: Option<usize>,
) {
    let hits = run(registry(), [(path, source.as_bytes())]);
    let actual: Vec<_> = hits
        .iter()
        .filter(|h| h.rule.strip_prefix("ts/") == Some(rule) || h.rule == "core/unparseable")
        .map(|h| (h.rule.strip_prefix("ts/").unwrap_or(h.rule), h.line, h.col))
        .collect();
    let expected: Vec<_> = offset
        .map(|i| {
            let prefix = &source[..i];
            (
                rule,
                1 + prefix.bytes().filter(|&c| c == b'\n').count(),
                1 + prefix.rsplit('\n').next().unwrap().len(),
            )
        })
        .into_iter()
        .collect();
    if actual != expected {
        missing.push(format!(
            "{label}: {actual:?} expected {expected:?} in {source}"
        ));
    }
}

#[test]
fn local_binding_spellings() {
    let mut missing = Vec::new();
    for source in [
        "const { shape } = schema;",
        "const shape = schema.shape;",
        "({ value: shape } = schema);",
        "({ shape } = schema);",
    ] {
        check_spelling(
            &mut missing,
            "local binding",
            "no-shape-in-symbol-names",
            source,
            source.find("shape"),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn type_queries_skip_external_member_names() {
    let mut missing = Vec::new();
    for (source, marker) in [
        ("type Field = typeof schema.shape;", None),
        ("type Field = typeof a.b.shape;", None),
        ("type Field = typeof a.shape.b;", None),
        ("type Field = typeof shapeSchema.x;", Some("shapeSchema")),
        ("type T = NS.Shape;", Some("Shape")),
        ("type MyShape = string;", Some("MyShape")),
        ("type F = typeof import(\"pkg\").shape;", None),
        ("type F = typeof import(\"pkg\").ns.shape;", None),
        ("type F = typeof import(\"pkg\").shape.x;", None),
        (
            "type F = typeof import(\"pkg\").ns.x<Shape>;",
            Some("Shape"),
        ),
        ("type F = import(\"pkg\").Shape;", Some("Shape")),
    ] {
        check_spelling(
            &mut missing,
            source,
            "no-shape-in-symbol-names",
            source,
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn computed_names_skip_values_shared_with_types() {
    let mut missing = Vec::new();
    check_spelling(
        &mut missing,
        "S118 computed name from a const sharing an interface name",
        "no-shape-in-symbol-names",
        "interface Keys { name: string } const Keys = { name: 'shape' }; const value = { [Keys.name]: 1 };",
        None,
    );
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn documented_slop_rule_limits() {
    let mut missing = Vec::new();
    for (label, rule, source) in [
        (
            "S74 empty object read in a comma expression",
            "no-conditional-empty-object-spread",
            "const empty = {}; (empty, log()); consume({ ...(flag ? empty : payload) });",
        ),
        (
            "empty object read as the final operand of a comma expression",
            "no-conditional-empty-object-spread",
            "const empty = {}; void (log(), empty); consume({ ...(flag ? empty : payload) });",
        ),
        (
            "S118 empty spread alias sharing a type name",
            "no-conditional-empty-object-spread",
            "declare const ok: boolean; type Empty = {}; const Empty = {}; const value = { ...(ok ? { a: 1 } : Empty) };",
        ),
    ] {
        check_spelling(&mut missing, label, rule, source, None);
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn untouched_const_owners_supply_member_values() {
    let mut missing = Vec::new();
    for (label, rule, source, marker) in [
        (
            "S49: call body writes its own selector",
            "no-conditional-empty-object-spread",
            "const names={method:'call'}; (()=>{ names.method='bind'; return {...value}; })[names.method](null); const value=ready?{ok:true}:{};",
            None,
        ),
        (
            "untouched computed name",
            "no-shape-in-symbol-names",
            "const p = {t: 'shape'}; consume({[p.t]: 0});",
            Some("p.t"),
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            rule,
            source,
            marker.and_then(|marker| source.find(marker)),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn template_keys_supply_member_values() {
    let mut missing = Vec::new();
    for (label, source) in [
        (
            "S72 template key read by dot access",
            "const box = {[`key`]: 'shape'}; consume({[box.key]: 0});",
        ),
        (
            "S72 template key read by bracket access",
            "const box = {[`key`]: 'shape'}; consume({[box['key']]: 0});",
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            source,
            Some(source.find("[box").unwrap() + 1),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn concatenation_budget_is_independent_of_key_order() {
    use std::fmt::Write as _;

    let rule = registry()
        .iter()
        .find(|r| r.id == "ts/no-shape-in-symbol-names")
        .unwrap();
    for (seed, fits, budget) in [
        ("", true, 32_772),
        ("xxx", false, 32_772),
        (r"\uDC00\uD800", true, 80_000),
    ] {
        let mut declarations = format!("const b0 = '{seed}';\n");
        let mut reads = String::new();
        for n in 1..=14 {
            let prev = n - 1;
            writeln!(declarations, "const b{n} = b{prev} + b{prev};").unwrap();
            writeln!(reads, "consume({{ [b{n}]: 0 }});").unwrap();
        }
        declarations.push_str("const suffix = 'sha' + 'pe';\n");
        let target = "consume({ [b14 + suffix]: 0 });\n";
        let standalone = "consume({ [suffix]: 0 });\n";
        for (keys, target_line, standalone_line) in [
            (format!("{target}{reads}{standalone}"), 17, 32),
            (format!("{standalone}{reads}{target}"), 32, 17),
        ] {
            let mut source = format!("{declarations}{keys}");
            source.extend(std::iter::repeat_n(' ', budget - source.len()));
            let hits = run(std::slice::from_ref(rule), [(TEST_FILE, source.as_bytes())]);
            let actual: Vec<_> = hits.iter().map(|h| (h.rule, h.line, h.col)).collect();
            let mut lines = vec![standalone_line];
            if fits {
                lines.push(target_line);
                lines.sort_unstable();
            }
            let expected: Vec<_> = lines
                .into_iter()
                .map(|line| ("ts/no-shape-in-symbol-names", line, 12))
                .collect();
            assert_eq!(actual, expected, "seed: {seed:?}, target: {target_line}");
        }
    }
}

#[test]
fn member_writes_hide_computed_names() {
    let mut missing = Vec::new();
    for (label, rule, source) in [
        (
            "removed symbol name",
            "no-shape-in-symbol-names",
            "const p={t:'shape'}; p.t='name'; consume({[p.t]:0});",
        ),
        (
            "introduced symbol name",
            "no-shape-in-symbol-names",
            "const p={t:'name'}; p.t='shape'; consume({[p.t]:0});",
        ),
        (
            "introduced name through a bracket write",
            "no-shape-in-symbol-names",
            "const p={t:'name'}; p['t']='shape'; consume({[p['t']]:0});",
        ),
        (
            "introduced name through an aliased owner",
            "no-shape-in-symbol-names",
            "const p={t:'name'}; const alias=p; alias.t='shape'; consume({[p.t]:0});",
        ),
    ] {
        check_spelling(&mut missing, label, rule, source, None);
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn written_selectors_are_unknown_at_immediate_calls() {
    let mut missing = Vec::new();
    for (label, rule, source) in [
        (
            "bound writer leaves reducer unchanged",
            "no-reduce-accumulator-copy",
            "const arr = [1]; const box={cb:other}; const names={method:'call'}; const alias=names; alias.method='bind'; (()=>{box.cb=(acc,x)=>({...acc,x});})[names.method](null); arr.reduce(box.cb,{});",
        ),
        (
            "bound writer leaves computed name unchanged",
            "no-shape-in-symbol-names",
            "const box={key:'name'}; const names={method:'call'}; names.method='bind'; (()=>{box.key='shape';})[names['method']](null); consume({[box.key]:0});",
        ),
        (
            "bound reader can see a later empty alias",
            "no-conditional-empty-object-spread",
            "const names={method:'call'}; names.method='bind'; (()=>({...value}))[names.method](null); const value=ready?{ok:true}:{};",
        ),
        (
            "immediate reader cannot see a later empty alias",
            "no-conditional-empty-object-spread",
            "const names={method:'bind'}; names.method='call'; (()=>({...value}))[names.method](null); const value=ready?{ok:true}:{};",
        ),
    ] {
        check_spelling(&mut missing, label, rule, source, None);
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn accessor_assignments_do_not_supply_member_values() {
    let mut missing = Vec::new();
    for (label, rule, source) in [
        (
            "callback through an alias of the owner",
            "no-reduce-accumulator-copy",
            "const arr = [1]; const box={get cb(){return other;},set cb(value){}}; const alias=box; alias.cb=(acc,x)=>({...acc,x}); arr.reduce(box.cb,{});",
        ),
        (
            "name with a bracket accessor",
            "no-shape-in-symbol-names",
            "const box={get ['key'](){return 'name';},set ['key'](value){}}; box['key']='shape'; consume({[box.key]:0});",
        ),
    ] {
        check_spelling(&mut missing, label, rule, source, None);
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn unknown_member_values_do_not_depend_on_read_order() {
    let mut missing = Vec::new();
    let rule = registry()
        .iter()
        .find(|r| r.id == "ts/no-shape-in-symbol-names")
        .unwrap();
    for (label, alias, right_first) in [
        ("member key first", "v", false),
        ("concatenated key first", "v", true),
        ("another alias before the member read", "alias", false),
    ] {
        let direct = "consume({[v]:0});";
        let joined = "consume({[key]:0});";
        let reads = if right_first {
            [joined, direct]
        } else {
            [direct, joined]
        };
        let [first, second] = reads;
        let source = format!(
            "const o={{k:'name'}}; while(more()) {{ o.k='shape'; const v=o.k; const alias=v; const key={alias}+''; o[key]='other';\n{first}\n{second}\n}}"
        );
        let hits = run(std::slice::from_ref(rule), [(TEST_FILE, source.as_bytes())]);
        let actual: Vec<_> = hits.iter().map(|h| (h.rule, h.line, h.col)).collect();
        let expected = [];
        if actual != expected {
            missing.push(format!("{label}: {actual:?} expected {expected:?}"));
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn wrapped_selectors_do_not_restore_written_members() {
    let mut missing = Vec::new();
    for (label, selector) in [
        ("as", "('t' as string)"),
        ("satisfies", "'t' satisfies string"),
        ("non-null", "'t'!"),
        ("parentheses", "(('t'))"),
        ("aliased selector", "field"),
    ] {
        let source = format!(
            "const p={{t:'name'}}; const field='t'; p.t='shape'; consume({{[p[{selector}]]:0}});"
        );
        check_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            &source,
            None,
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn wrapped_computed_names_fold_neutral_aliases() {
    let mut missing = Vec::new();
    for (label, source, marker) in [
        (
            "object key",
            "const token='shape'; const o={[(token)]:1};",
            "token)",
        ),
        (
            "class key",
            "const token='shape'; class C { [((token as string) satisfies string)!] = 1; }",
            "token as",
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            source,
            source.find(marker),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn accessor_prototype_and_spread_owners_are_unknown() {
    let mut missing = Vec::new();
    for (label, source) in [
        (
            "S56 own getter",
            "const box={key:'shape',get touch(){this.key='plain';}}; void box.touch; consume({[box.key]:0});",
        ),
        (
            "S56 own setter",
            "const box={key:'shape',set touch(value){this.key=value;}}; consume({[box.key]:0});",
        ),
        (
            "S56 inherited getter",
            "const proto={get touch(){this.key='plain';}}; const box={key:'shape',__proto__:proto}; void box.touch; consume({[box.key]:0});",
        ),
        (
            "S56 getter under computed literal key",
            "const box={key:'shape',get ['touch'](){this.key='plain';}}; void box.touch; consume({[box.key]:0});",
        ),
        (
            "S56 spread owner",
            "const box={...other,key:'shape'}; consume({[box.key]:0});",
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            source,
            None,
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn direct_eval_hides_member_and_const_string_values() {
    let mut missing = Vec::new();
    for (label, path, source) in [
        (
            "S57 direct eval before the read",
            TEST_FILE,
            "const box={key:'shape'}; eval(\"box.key='plain'\"); consume({[box.key]:0});",
        ),
        (
            "S57 direct eval inside a function",
            TEST_FILE,
            "const box={key:'shape'}; function mutate(){eval(\"box.key='plain'\");} mutate(); consume({[box.key]:0});",
        ),
        (
            "S57 direct eval and a bare const",
            TEST_FILE,
            "const key='shape'; eval(other); consume({[key]:0});",
        ),
        (
            "S57 references to a local eval binding hide values",
            TEST_FILE,
            "const eval=()=>{}; const box={key:'shape'}; eval(other); consume({[box.key]:0});",
        ),
        (
            "S57 optional eval hides values",
            TEST_FILE,
            "const box={key:'shape'}; eval?.(other); consume({[box.key]:0});",
        ),
        (
            "S57 ambient function named eval",
            "case.cts",
            "declare function eval(source: string): void; const box={key:'shape'}; eval(\"box.key='plain'\"); consume({[box.key]:0});",
        ),
        (
            "S57 ambient const named eval",
            TEST_FILE,
            "declare const eval: (s: string) => void; const box={key:'shape'}; eval(\"box.key='plain'\"); consume({[box.key]:0});",
        ),
        (
            "S57 references to a runtime function named eval hide values",
            "case.cts",
            "function eval() {} const box={key:'shape'}; eval(other); consume({[box.key]:0});",
        ),
        (
            "S61 eval merged with a namespace containing a const enum hides values",
            "case.cts",
            "declare function eval(source: string): void; namespace eval { export const enum E { item } } const box={key:'shape'}; eval(\"box.key='plain'\"); consume({[box.key]:0});",
        ),
        (
            "S62 namespace reference to an ambient eval hides values",
            "case.cts",
            "function eval() {} const box={key:'shape'}; namespace N { declare function eval(s: string): void; eval(other); } consume({[box.key]:0});",
        ),
        (
            "an eval alias hides values",
            TEST_FILE,
            "const box={key:'shape'}; const e = eval; e(\"box.key='plain'\"); consume({[box.key]:0});",
        ),
        (
            "a type-only typeof eval hides values",
            TEST_FILE,
            "const box={key:'shape'}; const other: typeof eval = fn; consume({[box.key]:0});",
        ),
    ] {
        check_file_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            (path, source),
            None,
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

const EVAL_MENTION_SCRIPTS: [(&str, &str, bool); 8] = [
    (
        "S67 dot access on globalThis",
        r#"globalThis.eval("box.key='plain'");"#,
        false,
    ),
    (
        "S67 string key on globalThis",
        r#"globalThis['eval']("box.key='plain'");"#,
        false,
    ),
    (
        "S67 template key on globalThis",
        r#"globalThis[`eval`]("box.key='plain'");"#,
        false,
    ),
    (
        "S67 dot access on window",
        r#"window.eval("box.key='plain'");"#,
        false,
    ),
    (
        "S67 destructured identifier key",
        r#"const {eval: run} = globalThis; run("box.key='plain'");"#,
        false,
    ),
    (
        "S67 destructured string key",
        r#"const {'eval': run} = globalThis; run("box.key='plain'");"#,
        false,
    ),
    ("an unrelated eval key", "const o = { eval: 1 };", false),
    (
        "dynamic code without an eval mention still folds",
        r#"new Function("box.key='plain'")();"#,
        true,
    ),
];

#[test]
fn every_eval_mention_hides_member_and_const_string_values() {
    let mut missing = Vec::new();
    for (label, declaration) in [
        (
            "S63 declare class",
            "declare class Derived extends base(eval) {}",
        ),
        (
            "S63 exported declare class",
            "export declare class Derived extends base(eval) {}",
        ),
        (
            "S63 declare namespace",
            "declare namespace N { class Derived extends base(eval) {} }",
        ),
        (
            "S63 declare module",
            "declare module \"m\" { class Derived extends base(eval) {} }",
        ),
        (
            "S63 declare global",
            "export {}; declare global { class Derived extends base(eval) {} }",
        ),
        ("S63 runtime class", "class Derived extends base(eval) {}"),
        (
            "S63 runtime namespace",
            "namespace N { class Derived extends base(eval) {} }",
        ),
        (
            "S64 class method overload",
            "class C { [eval](): void; [key]() {} }",
        ),
        (
            "S64 function overload",
            "function f({[eval]: {value}}: {x: {value: string}}): void; function f(x: unknown) {}",
        ),
        (
            "S64 abstract method",
            "abstract class C { abstract [eval](): void; }",
        ),
        (
            "S64 abstract property",
            "abstract class C { abstract [eval]: string; }",
        ),
        (
            "S64 abstract getter",
            "abstract class C { abstract get [eval](): string; }",
        ),
        (
            "S64 abstract auto-accessor",
            "abstract class C { abstract accessor [eval]: string; }",
        ),
        (
            "S64 static method overload",
            "class C { static [eval](): void; static [key]() {} }",
        ),
        ("S64 method implementation", "class C { [eval]() {} }"),
        ("S64 plain field", "class C { [eval]: string; }"),
        ("S64 object method", "const obj = { [eval]() {} };"),
        ("S64 default parameter", "function f(x = eval) {}"),
        (
            "S65 interface method destructuring key",
            "interface I { m({[eval]: {value}}: {x: {value: string}}): void }",
        ),
        (
            "S66 const enum initializer",
            "const eval = 'x'; const enum E { Item = eval } const v = E.Item;",
        ),
        ("a type reference to eval", "type Alias = eval;"),
    ] {
        let source = format!("{declaration} const box={{key:'shape'}}; consume({{[box.key]:0}});");
        check_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            &source,
            None,
        );
    }

    for (label, statement, keeps_finding) in EVAL_MENTION_SCRIPTS {
        let source = format!("const box={{key:'shape'}}; {statement} consume({{[box.key]:0}});");
        check_file_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            ("case.js", &source),
            source.find("box.key]").filter(|_| keeps_finding),
        );
    }

    let control = "const box={key:'shape'}; consume({[box.key]:0});";
    check_spelling(
        &mut missing,
        "a file without eval preserves values",
        "no-shape-in-symbol-names",
        control,
        Some(control.find("box.key]").unwrap()),
    );
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn jsx_member_checks_only_the_root_name() {
    let mut missing = Vec::new();
    for (label, source, columns) in [
        ("S73 member", "<Vendor.shape />;", &[][..]),
        ("S73 nested member", "<Vendor.Shape.item />;", &[][..]),
        (
            "S73 local root",
            "const localShape = X; <localShape.shape />;",
            &[7, 24][..],
        ),
    ] {
        let hits = run(registry(), [("case.tsx", source.as_bytes())]);
        let actual: Vec<_> = hits
            .iter()
            .filter(|h| matches!(h.rule, "ts/no-shape-in-symbol-names" | "core/unparseable"))
            .map(|h| (h.rule, h.line, h.col))
            .collect();
        let expected: Vec<_> = columns
            .iter()
            .map(|&column| ("ts/no-shape-in-symbol-names", 1, column))
            .collect();
        if actual != expected {
            missing.push(format!("{label}: {actual:?} expected {expected:?}"));
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn jsx_character_references_spelling_eval_keep_values() {
    let mut missing = Vec::new();
    let jsx =
        "const box={key:'shape'}; const view = <X attr=\"ev&#x61;l\" />; consume({[box.key]:0});";
    check_file_spelling(
        &mut missing,
        "S68 a JSX character reference spelling eval still folds",
        "no-shape-in-symbol-names",
        ("case.tsx", jsx),
        Some(jsx.find("box.key]").unwrap()),
    );
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn with_bodies_hide_member_and_const_string_values() {
    let mut missing = Vec::new();
    for (label, source, marker) in [
        (
            "S58 member read inside with",
            "const box={key:'shape'}; with ({box:{key:'plain'}}) { consume({[box.key]:0}); }",
            None,
        ),
        (
            "S58 bare const inside with",
            "const key='shape'; with ({key:'plain'}) { consume({[key]:0}); }",
            None,
        ),
        (
            "S58 member read after with",
            "const box={key:'shape'}; with ({box:{key:'plain'}}) { consume({[box.key]:0}); } consume({[box.key]:0});",
            Some("box.key]"),
        ),
        (
            "S58 member read in the with object expression",
            "const box={key:'shape'}; with ({[box.key]:0}) {}",
            Some("box.key]"),
        ),
    ] {
        check_file_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            ("case.cjs", source),
            marker.and_then(|marker| source.rfind(marker)),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn only_plain_safe_integer_keys_supply_numeric_members() {
    let mut missing = Vec::new();
    for (label, source, marker) in [
        (
            "S60 exponent key",
            "const box={'1000000000000000000000':'plain',1e21:'shape'}; consume({[box['1000000000000000000000']]:0});",
            None,
        ),
        (
            "S60 fractional key",
            "const box={'0.1':'plain',0.1:'shape'}; consume({[box['0.1']]:0});",
            None,
        ),
        (
            "S60 separated key",
            "const box={'1000':'plain',1_000:'shape'}; consume({[box['1000']]:0});",
            None,
        ),
        (
            "S60 unsafe integer key",
            "const box={9007199254740992:'shape'}; consume({[box['9007199254740992']]:0});",
            None,
        ),
        (
            "S60 plain safe integer key",
            "const box={9007199254740991:'shape'}; consume({[box['9007199254740991']]:0});",
            Some("box["),
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            source,
            marker.and_then(|marker| source.find(marker)),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn commonjs_numeric_keys_require_plain_decimal_spelling() {
    let mut missing = Vec::new();
    for (key, selector, keeps_finding) in [
        ("010", "8", false),
        ("08", "8", false),
        ("9007199254740993n", "9007199254740993", false),
        ("0", "0", true),
        ("7", "7", true),
    ] {
        let source = format!("const box={{{key}:'shape'}}; consume({{[box['{selector}']]:0}});");
        check_file_spelling(
            &mut missing,
            key,
            "no-shape-in-symbol-names",
            ("case.cjs", &source),
            source.find("box[").filter(|_| keeps_finding),
        );
    }
    assert!(missing.is_empty(), "S60\n{}", missing.join("\n"));
}

#[test]
fn numeric_member_selectors_use_plain_safe_decimal_keys() {
    let mut missing = Vec::new();
    for (owner_key, selector, reports) in [
        ("0", "0", true),
        ("123", "(123 as number)", true),
        ("123", "(sideEffect(), 123)", true),
        ("9007199254740991", "9007199254740991", true),
        ("'9007199254740992'", "9007199254740992", false),
        ("'1000'", "1_000", false),
        ("'1000'", "1e3", false),
        ("'16'", "0x10", false),
        ("'1.5'", "1.5", false),
        ("'0'", "-0", false),
    ] {
        let source = format!(
            "const names = {{ {owner_key}: 'userShape' }};\nconsume({{ [names[{selector}]]: 0 }});"
        );
        check_spelling(
            &mut missing,
            selector,
            "no-shape-in-symbol-names",
            &source,
            reports.then(|| source.find("names[").unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn slop_spellings() {
    let mut missing = Vec::new();
    for &(label, wrapper) in WRAPPERS {
        for (slot, source, value, rule, report) in [
            (
                "spread argument",
                "consume({...VALUE});",
                "(c ? {} : x)",
                "no-conditional-empty-object-spread",
                true,
            ),
            (
                "conditional arm",
                "consume({...(c ? VALUE : x)});",
                "{}",
                "no-conditional-empty-object-spread",
                true,
            ),
            (
                "early read",
                "consume({...(c ? VALUE : x)});const empty={};",
                "empty",
                "no-conditional-empty-object-spread",
                false,
            ),
            (
                "initialized read",
                "const empty={};consume({...(c ? VALUE : x)});",
                "empty",
                "no-conditional-empty-object-spread",
                label != "comma",
            ),
        ] {
            let source = source.replace("VALUE", &wrapper.replace("VALUE", value));
            check_spelling(
                &mut missing,
                &format!("{slot} {label}"),
                rule,
                &source,
                report.then(|| source.find("...").unwrap()),
            );
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn jsx_member_tags_keep_const_member_values() {
    let mut missing = Vec::new();
    for tag in [
        "<UI.Section />",
        "<UI.Components.Section></UI.Components.Section>",
    ] {
        let source = format!(
            "const Panel = () => <section />;\nconst UI = {{ Section: Panel, key: 'userShape' }};\nconst view = {tag};\nconst value = {{ [UI.key]: 1 }};"
        );
        check_file_spelling(
            &mut missing,
            tag,
            "no-shape-in-symbol-names",
            ("case.tsx", &source),
            source.find("UI.key"),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn type_only_exports_keep_const_member_values() {
    let mut missing = Vec::new();
    for (export, reports) in [
        ("export type { names };", true),
        ("export { type names };", true),
        ("export { type names, other };", true),
        ("export { names, type other };", false),
    ] {
        let source = format!(
            "const names = {{ key: 'userShape' }}; const other = 0;\n{export}\nconsume({{ [names.key]: 0 }});"
        );
        check_spelling(
            &mut missing,
            export,
            "no-shape-in-symbol-names",
            &source,
            reports.then(|| source.find("names.key").unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn computed_member_selectors_preserve_unicode_keys() {
    let mut missing = Vec::new();
    for (label, selector, reports) in [
        ("paired surrogates", r"'\uD83D' + '\uDE00'", false),
        ("lone surrogate", r"'\uD83D' + '!'", false),
        ("incomplete key", r"'\uD83D' + '?'", false),
        ("longer key", r"'\uD83D' + '\uDE00' + '!'", false),
        ("literal with paired surrogates", "'😀'", true),
        ("literal with lone surrogate", r"'\uD83D!'", true),
        (
            "wrapped template alias",
            "(sideEffect(), ((key as string) satisfies string)!)",
            true,
        ),
        ("concatenated alias", "joined", false),
        ("field read", "parts.key", false),
    ] {
        let source = format!(
            r"const names = {{ '😀': 'plain', ['\uD83D\uDE00']: 'userShape', '\uD83D!': 'listShape' }};
const key = (`😀` as string);
const joined = '' + key;
const parts = {{ key }};
consume({{ [names[{selector}]]: 0 }});"
        );
        check_spelling(
            &mut missing,
            label,
            "no-shape-in-symbol-names",
            &source,
            reports.then(|| source.find("names[").unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn conditional_empty_spreads_skip_early_reads_through_reassigned_members() {
    let source = "const methods = { run: Reflect.apply };
methods.run = () => {};
methods.run(() => ({ ...value }), null, []);
const value = ready ? {} : payload;";
    let mut missing = Vec::new();
    check_spelling(
        &mut missing,
        "reassigned member with an early reader",
        "no-conditional-empty-object-spread",
        source,
        None,
    );
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn conditional_empty_spreads_hide_aliases_in_dynamic_scope() {
    let mut missing = Vec::new();
    for (label, source, marker) in [
        (
            "S103 with shadows an empty object alias",
            "const empty = {}; with ({ empty: { n: 1 } }) { const result = { ...(true ? empty : { n: 2 }) }; }",
            None,
        ),
        (
            "S103 eval mutates an empty object alias",
            "const empty = {}; eval('empty.n = 1'); const result = { ...(true ? empty : { n: 2 }) };",
            None,
        ),
        (
            "S103 with keeps an empty object literal",
            "with (scope) { const result = { ...(c ? {} : x) }; }",
            Some("..."),
        ),
        (
            "S103 eval keeps an empty object literal",
            "eval(source); const result = { ...(c ? {} : x) };",
            Some("..."),
        ),
    ] {
        check_file_spelling(
            &mut missing,
            label,
            "no-conditional-empty-object-spread",
            ("case.cjs", source),
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn explicit_pipeline_receivers() {
    let mut missing = Vec::new();
    for (rule, source, marker) in [
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; (0, xs.reduce)(acc => ({ ...acc }), {});",
            Some("...acc"),
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; import {assign} from 'Object'; xs.reduce(acc=>assign({},acc),{});",
            None,
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; import * as other from 'Array'; xs.reduce(acc=>other.from(acc),[]);",
            None,
        ),
        (
            "no-reduce-accumulator-copy",
            "const page = { rows: [1], other: {} }; page.rows.reduce.call(page.other, acc => [...acc], []);",
            None,
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; xs.reduce(acc=>acc.concat.call(other,n),[]);",
            None,
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; xs.reduce(acc=>Array.prototype.concat.call(acc,n),[]);",
            Some("Array.prototype"),
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; xs.reduce(acc=>acc.concat.call(acc,n),[]);",
            Some("acc.concat"),
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; xs.reduce.bind(xs,acc=>Object.assign({},acc),{})();",
            Some("Object.assign"),
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; xs.reduce.bind(xs,acc=>Object.assign({},acc),{});",
            None,
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; const Array=custom; xs.reduce(acc=>globalThis.Array.from(acc),[]);",
            Some("globalThis"),
        ),
        (
            "no-reduce-accumulator-copy",
            "const xs = [1]; const globalThis=custom; xs.reduce(acc=>globalThis.Array.from(acc),[]);",
            None,
        ),
    ] {
        check_spelling(
            &mut missing,
            "receiver",
            rule,
            source,
            marker.map(|m: &str| source.find(m).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn annotated_bindings_supply_array_evidence() {
    let mut missing = Vec::new();
    for (label, rule, source, marker) in [
        (
            "S114 unseeded reduce with an array accumulator",
            "no-reduce-accumulator-copy",
            "[[1], [2]].reduce((acc: number[], n) => acc.concat(n));",
            Some("acc.concat"),
        ),
        (
            "S114 unseeded reduce without an annotation",
            "no-reduce-accumulator-copy",
            "[[1], [2]].reduce((acc, n) => acc.concat(n));",
            None,
        ),
        (
            "S116 reducer seed reached through a member",
            "no-reduce-accumulator-copy",
            "declare function load(): number[]; const rows: number[] = load(); const box = { rows }; [[1]].reduce((acc, n) => acc.concat(n), box.rows);",
            Some("acc.concat"),
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            rule,
            source,
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn for_await_loops_suspend_async_regions() {
    let mut missing = Vec::new();
    for (label, rule, source, marker) in [
        (
            "S117 reducer copy inside a for await loop",
            "no-reduce-accumulator-copy",
            "declare const items: AsyncIterable<number>; [1].reduce((acc: number[], n) => (async () => { for await (const x of items) { return [...acc, n]; } })(), []);",
            None,
        ),
        (
            "S117 reducer copy before a for await loop",
            "no-reduce-accumulator-copy",
            "declare const items: AsyncIterable<number>; [1].reduce((acc: number[], n) => (async () => { const c = [...acc, n]; for await (const x of items) {} return c; })(), []);",
            Some("...acc"),
        ),
        (
            "S117 reducer copy in a for await head default",
            "no-reduce-accumulator-copy",
            "declare const items: AsyncIterable<{ a?: number[] }>; [1].reduce((acc: number[], n) => (async () => { for await (const { a = [...acc, n] } of items) { return a; } })(), []);",
            None,
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            rule,
            source,
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn suspensions_split_every_region() {
    let mut missing = Vec::new();
    check_spelling(
        &mut missing,
        "S126 reducer copy after an optional chain base",
        "no-reduce-accumulator-copy",
        "declare const ready: Promise<number>; [1].reduce((acc: number[], n) => (async () => { (await ready)?.toString(); return [...acc, n]; })(), []);",
        None,
    );
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn generator_bodies_and_async_disposal_split_regions() {
    let mut missing = Vec::new();
    for (label, rule, source, marker) in [
        (
            "S134 inline generator reducer",
            "no-reduce-accumulator-copy",
            "[1].reduce(function* (acc: number[], n) { yield* [...acc, n]; }, []);",
            None,
        ),
        (
            "S134 declared generator reducer",
            "no-reduce-accumulator-copy",
            "function* step(acc: number[], n: number) { yield* [...acc, n]; } [1].reduce(step, []);",
            None,
        ),
        (
            "S134 plain reducer",
            "no-reduce-accumulator-copy",
            "[1].reduce(function (acc: number[], n) { return [...acc, n]; }, []);",
            Some("...acc"),
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            rule,
            source,
            marker.and_then(|marker| source.find(marker)),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn array_evidence_follows_with() {
    let mut missing = Vec::new();
    let source = "const xs = [1]; xs.reduce((acc, n) => acc.concat(n), [0].with(0, 1));";
    check_spelling(
        &mut missing,
        "S79 reducer seeded with with",
        "no-reduce-accumulator-copy",
        source,
        source.find("acc.concat"),
    );
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn documented_reducer_limits() {
    let mut missing = Vec::new();
    for (label, rule, source, marker) in [
        (
            "S75 copy inside an immediately constructed function",
            "no-reduce-accumulator-copy",
            "[1].reduce(acc => { new (function () { Object.assign({}, acc); })(); return acc; }, {});",
            None,
        ),
        (
            "S110 logical callee",
            "no-reduce-accumulator-copy",
            "declare const callback: (() => number[]) | undefined; [1].reduce((acc: number[], n) => (callback ?? (() => [...acc, n]))(), []);",
            None,
        ),
        (
            "S151 Object.assign source after a spread argument",
            "no-reduce-accumulator-copy",
            "declare const layers: object[]; [1].reduce(acc => Object.assign({}, ...layers, acc), {});",
            None,
        ),
        (
            "overloaded reducer implementation copies its accumulator",
            "no-reduce-accumulator-copy",
            "function step(acc: number[], n: number): number[]; function step(acc: number[], n: number | string): number[]; function step(acc: number[], n: number | string) { return [...acc, Number(n)]; } [1].reduce(step, []);",
            Some("...acc"),
        ),
        (
            "redeclared reducer variable stays unresolved",
            "no-reduce-accumulator-copy",
            "var step = (acc: number[], n: number) => [...acc, n]; var step = (acc: number[], n: number) => acc; [1].reduce(step, []);",
            None,
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            rule,
            source,
            marker.and_then(|marker| source.find(marker)),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

const REDUCER_CALLS: &[Row] = &[
    Row {
        rule: "no-reduce-accumulator-copy",
        path: "xs.reduce",
        args: "acc=>acc.concat(1),[]",
        prelude: "const xs=[1];",
        context: "\nCALL",
        module: "",
        options: "",
        callback: "acc=>acc.concat(1)",
    },
    Row {
        rule: "no-reduce-accumulator-copy",
        path: "Object.assign",
        args: "{},acc",
        prelude: "",
        context: "const items = [1]; items.reduce(acc=>{DECL return (\nCALL)}, {})",
        module: "",
        options: "",
        callback: "",
    },
    Row {
        rule: "no-reduce-accumulator-copy",
        path: "Array.from",
        args: "acc",
        prelude: "",
        context: "const items = [1]; items.reduce(acc=>{DECL return (\nCALL)}, [])",
        module: "",
        options: "",
        callback: "",
    },
    Row {
        rule: "no-reduce-accumulator-copy",
        path: "acc.concat",
        args: "n",
        prelude: "const n=1;",
        context: "const items = [1]; items.reduce(acc=>{DECL return (\nCALL)}, [])",
        module: "",
        options: "",
        callback: "",
    },
];

#[test]
fn reducer_spellings() {
    let mut missing = Vec::new();
    for row in REDUCER_CALLS {
        for (label, source) in cases(row) {
            let start = if row.path == "xs.reduce" {
                source.find("acc.concat").unwrap()
            } else {
                source.find('\n').unwrap() + 1 + usize::from(label == "whole call")
            };
            check_spelling(
                &mut missing,
                &format!("{} {label}", row.path),
                row.rule,
                &source,
                Some(start),
            );
        }
    }
    for &(label, wrapper) in WRAPPERS {
        for (slot, source, value, rule, report) in [
            (
                "object spread",
                "const items = [1]; items.reduce(acc=>({...VALUE}),{});",
                "acc",
                "no-reduce-accumulator-copy",
                true,
            ),
            (
                "array spread",
                "const items = [1]; items.reduce(acc=>[...VALUE],[]);",
                "acc",
                "no-reduce-accumulator-copy",
                true,
            ),
        ] {
            let source = source.replace("VALUE", &wrapper.replace("VALUE", value));
            check_spelling(
                &mut missing,
                &format!("{slot} {label}"),
                rule,
                &source,
                report.then(|| source.find("...").unwrap()),
            );
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn conditional_callees_leave_reducer_copies_unreported() {
    let mut missing = Vec::new();
    for source in [
        "[1].reduce((acc, n) => (flag ? () => [...acc, n] : () => [n])(), []);",
        "[1].reduce((acc, n) => (flag ? () => [n] : (() => [...acc, n]))(), []);",
        "[1].reduce((acc, n) => ((() => [...acc, n]) ? () => [] : () => [])(), []);",
        "[1].reduce((acc, n) => { const f = flag ? () => [...acc, n] : () => [n]; return f(); }, []);",
        "[1].reduce((acc, n) => invoke(flag ? () => [...acc, n] : () => [n]), []);",
    ] {
        check_spelling(
            &mut missing,
            source,
            "no-reduce-accumulator-copy",
            source,
            None,
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn reducer_self_call_defaults_keep_the_accumulator() {
    let mut missing = Vec::new();
    for (label, source, marker) in [
        (
            "S112 self call with defaults",
            "[0, 1].reduce(function fold(acc = [0], n = 1) { return n ? [...acc, n] : fold(); }, []);",
            "...acc",
        ),
        (
            "S113 alias of a defaulted accumulator",
            "[0, 1].reduce(function fold(acc = {}, n = 1) { const prev = acc; return n ? Object.assign({}, prev) : fold(); }, {});",
            "Object.assign",
        ),
        (
            "S113 assigned alias of a defaulted accumulator",
            "[0, 1].reduce(function fold(acc = {}, n = 1) { let prev; prev = acc; return n ? Object.assign({}, prev) : fold(); }, {});",
            "Object.assign",
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            "no-reduce-accumulator-copy",
            source,
            Some(source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn names_shared_with_types_give_no_values() {
    let mut missing = Vec::new();
    for (label, rule, source) in [
        (
            "S130 reducer seed annotated with a type declared before a value",
            "no-reduce-accumulator-copy",
            "type Rows = number[]; const Rows = 0; declare const seed: Rows; [1].reduce((acc, n) => acc.concat(n), seed);",
        ),
        (
            "S131 copy target sharing a type name",
            "no-reduce-accumulator-copy",
            "[1].reduce(acc => { type Target = {}; const Target = {}; return Object.assign(Target, acc); }, {});",
        ),
        (
            "S131 type alias sharing a var with redeclarations",
            "no-reduce-accumulator-copy",
            "[1].reduce(acc => { type Target = object; var Target: Target = {}; var Target: Target; return Object.assign(Target, acc); }, {});",
        ),
        (
            "S131 interface sharing a local const",
            "no-reduce-accumulator-copy",
            "[1].reduce(acc => { const Target: Target = {}; interface Target {} return Object.assign(Target, acc); }, {});",
        ),
    ] {
        check_spelling(&mut missing, label, rule, source, None);
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn accumulator_writes_shared_names_and_switch_cases() {
    let mut missing = Vec::new();
    for (label, rule, source, marker) in [
        (
            "S138 reducer copy after await using in a sole default clause",
            "no-reduce-accumulator-copy",
            "declare const res: AsyncDisposable; [1].reduce((acc: number[], n) => (async () => { switch (0) { default: { await using r = res; } } return [...acc, n]; })(), []);",
            None,
        ),
        (
            "S139 spread after reassigning the accumulator",
            "no-reduce-accumulator-copy",
            "[[1], [2]].reduce((acc, item) => { acc = item; return [...acc]; }, []);",
            None,
        ),
        (
            "S139 assign after reassigning the accumulator",
            "no-reduce-accumulator-copy",
            "[{}].reduce((acc, item) => { acc = item; return Object.assign({}, acc); }, {});",
            None,
        ),
        (
            "S139 alias copied after reassignment",
            "no-reduce-accumulator-copy",
            "[[1], [2]].reduce((acc, item) => { const prev = acc; acc = item; return [...prev]; }, []);",
            None,
        ),
        (
            "S139 copy before reassignment",
            "no-reduce-accumulator-copy",
            "[1].reduce((acc: number[], n) => { const next = [...acc, n]; acc = next; return acc; }, []);",
            Some("...acc"),
        ),
        (
            "S140 accumulator sharing a type alias name",
            "no-reduce-accumulator-copy",
            "[1].reduce((acc: number[], n) => { type acc = number[]; return [...acc, n]; }, []);",
            None,
        ),
        (
            "S140 accumulator sharing an interface name",
            "no-reduce-accumulator-copy",
            "[1].reduce((acc: number[], n) => { interface acc {} return [...acc, n]; }, []);",
            None,
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            rule,
            source,
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn assignments_immediate_calls_and_first_case_tests() {
    let mut missing = Vec::new();
    for (label, rule, source, marker) in [
        (
            "S141 spread assigned back to the accumulator",
            "no-reduce-accumulator-copy",
            "[1].reduce((acc: number[], n) => { acc = [...acc, n]; return acc; }, []);",
            Some("...acc"),
        ),
        (
            "S141 assign assigned back to the accumulator",
            "no-reduce-accumulator-copy",
            "[{}].reduce((acc, item) => { acc = Object.assign({}, acc, item); return acc; }, {});",
            Some("Object.assign"),
        ),
        (
            "S142 copy in an immediately called arrow before reassignment",
            "no-reduce-accumulator-copy",
            "[1].reduce((acc: number[], n) => { const next = (() => [...acc, n])(); acc = next; return acc; }, []);",
            Some("...acc"),
        ),
        (
            "S142 reassignment inside an immediately called arrow",
            "no-reduce-accumulator-copy",
            "[[1]].reduce((acc, item) => (() => { acc = item; return [...acc]; })(), []);",
            None,
        ),
        (
            "S145 a wrapped accumulator target silences the callback",
            "no-reduce-accumulator-copy",
            "[1].reduce((acc: number[], n) => { (acc as number[]) = [...acc, n]; return acc; }, []);",
            None,
        ),
        (
            "S145 a non-null accumulator target silences the callback",
            "no-reduce-accumulator-copy",
            "[1].reduce((acc: number[], n) => { acc! = [...acc, n]; return acc; }, []);",
            None,
        ),
        (
            "S147 accumulator replaced inside a computed target key",
            "no-reduce-accumulator-copy",
            "declare const out: number[][]; [[1]].reduce((acc, item) => { out[(() => { for (acc of [item]) {} return 0; })()] = [...acc]; return acc; }, []);",
            None,
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            rule,
            source,
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn nested_case_awaits_and_array_from_evidence() {
    let mut missing = Vec::new();
    for (label, rule, source, marker) in [
        (
            "S149 Array.from of an accumulator without array evidence",
            "no-reduce-accumulator-copy",
            "[1].reduce(acc => { Array.from(acc); return acc; }, { length: 0 });",
            None,
        ),
        (
            "S149 Array.from of an accumulator seeded with an array",
            "no-reduce-accumulator-copy",
            "[1].reduce(acc => { Array.from(acc); return acc; }, []);",
            Some("Array.from"),
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            rule,
            source,
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn reducer_assign_targets_require_declaration_initializers() {
    let mut missing = Vec::new();
    for (label, setup, target, report) in [
        (
            "conditional let assignment",
            "let t; if (enabled) t = {};",
            "t",
            false,
        ),
        (
            "assignment after a branch",
            "let t; if (enabled) log(); t = {};",
            "t",
            false,
        ),
        (
            "assignment in both branches",
            "let t; if (enabled) t = {}; else t = {};",
            "t",
            false,
        ),
        (
            "conditional var assignment",
            "var t; if (enabled) t = {};",
            "t",
            false,
        ),
        (
            "conditional var declaration",
            "if (enabled) { var t = {}; }",
            "t",
            false,
        ),
        (
            "alias of a conditionally assigned target",
            "let t; if (enabled) t = {}; const alias = t;",
            "alias",
            false,
        ),
        (
            "S111 fresh local reached through a member",
            "const t = {}; const holder = { target: t };",
            "holder.target",
            false,
        ),
        (
            "fresh literal member",
            "const holder = { target: {} };",
            "holder.target",
            true,
        ),
        ("initialized let", "let t = {};", "t", true),
        ("initialized var", "var t = {};", "t", true),
    ] {
        let source =
            format!("[1].reduce(acc => {{ {setup} return Object.assign({target}, acc); }}, {{}});");
        check_spelling(
            &mut missing,
            label,
            "no-reduce-accumulator-copy",
            &source,
            report.then(|| source.find("Object.assign").unwrap()),
        );
    }
    for (label, source) in [
        (
            "with can shadow an initialized var",
            "var o = { t: shared }; [1].reduce(function (acc) { var t = {}; with (o) { return Object.assign(t, acc); } }, {});",
        ),
        (
            "with can shadow an initialized let",
            "var o = { t: shared }; [1].reduce(function (acc) { let t = {}; with (o) { return Object.assign(t, acc); } }, {});",
        ),
        (
            "member of a conditionally assigned target",
            "[1].reduce(acc => { let o; if (c) o = { t: {} }; return Object.assign(o.t, acc); }, {});",
        ),
    ] {
        check_file_spelling(
            &mut missing,
            label,
            "no-reduce-accumulator-copy",
            ("case.cjs", source),
            None,
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn fresh_reducer_targets_follow_their_declarators() {
    let mut missing = Vec::new();
    for (label, source, report) in [
        (
            "plain var in a function body",
            "[1].reduce(function (acc) { var t = {}; return Object.assign(t, acc); }, {});",
            true,
        ),
        (
            "S86 labeled var in a function body",
            "[1].reduce(function (acc) { fresh: var t = {}; return Object.assign(t, acc); }, {});",
            true,
        ),
        (
            "S86 var in the same switch case",
            "[1].reduce(acc => { switch (mode) { case 0: var t = {}; return Object.assign(t, acc); } }, {});",
            true,
        ),
        (
            "S86 var in a for initializer",
            "[1].reduce(acc => { for (var t = {}; enabled;) { return Object.assign(t, acc); } }, {});",
            true,
        ),
        (
            "S91 var initializer in a for-in binding",
            "[1].reduce(function (acc) { for (var t = {} in { key: 1 }) return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S93 using target",
            "[1].reduce(acc => { using t = { [Symbol.dispose]() {} }; return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S93 await using target",
            "[1].reduce(async acc => { await using t = { [Symbol.asyncDispose]() {} }; return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "plain assignment",
            "[1].reduce(acc => { let t; t = {}; return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "declarator followed by a second write",
            "[1].reduce(acc => { let t = {}; t = {}; return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "read in an immediately invoked nested function",
            "[1].reduce(acc => { const t = {}; return (() => Object.assign(t, acc))(); }, {});",
            false,
        ),
        (
            "alias of a fresh declarator",
            "[1].reduce(acc => { const t = {}; const alias = t; return Object.assign(alias, acc); }, {});",
            false,
        ),
        (
            "read in a nested block",
            "[1].reduce(acc => { let t = {}; if (enabled) { return Object.assign(t, acc); } }, {});",
            true,
        ),
        (
            "read in a different switch case",
            "[1].reduce(acc => { switch (mode) { case 0: var t = {}; case 1: return Object.assign(t, acc); } }, {});",
            false,
        ),
        (
            "var under an unbraced condition",
            "[1].reduce(function (acc) { if (enabled) var t = {}; return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "var under an unbraced condition read from the else branch",
            "[1].reduce((acc, x) => { if (x) var t = {}; else return Object.assign(t, acc); return acc; }, {})",
            false,
        ),
        (
            "labeled var under an unbraced condition read from the else branch",
            "[1].reduce((acc, x) => { if (x) l: var t = {}; else return Object.assign(t, acc); return acc; }, {})",
            false,
        ),
        (
            "var in a labeled block read after it",
            "[1].reduce(function (acc) { fresh: { var t = {}; } return Object.assign(t, acc); }, {});",
            false,
        ),
    ] {
        check_file_spelling(
            &mut missing,
            label,
            "no-reduce-accumulator-copy",
            ("case.cjs", source),
            report.then(|| source.find("Object.assign").unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn fresh_reducer_targets_allow_bare_var_redeclarations() {
    let mut missing = Vec::new();
    for (label, source, report) in [
        (
            "S94 initialized var followed by a bare redeclaration",
            "[1].reduce(acc => { var t = {}; var t; return Object.assign(t, acc); }, {});",
            true,
        ),
        (
            "S94 bare var followed by an initialized redeclaration",
            "[1].reduce(acc => { var t; var t = {}; return Object.assign(t, acc); }, {});",
            true,
        ),
        (
            "S94 two initialized redeclarations",
            "[1].reduce(acc => { var t = {}; var t = {}; return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S94 redeclaration with a separate assignment",
            "[1].reduce(acc => { var t = {}; var t; t = {}; return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S95 block function overwrites the var binding",
            "[1].reduce(acc => { var t = {}; var t; if (true) { function t() {} } return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S98 static block function leaves the outer var fresh",
            "[1].reduce(acc => { var t = {}; class C { static { function t() {} } } return Object.assign(t, acc); }, {});",
            true,
        ),
        (
            "S102 nested function obscures a var in the same static block",
            "[1].reduce(acc => { class C { static { var t = {}; { function t() {} } Object.assign(t, acc); } } }, {});",
            false,
        ),
        (
            "S94 redeclaration bound by iteration",
            "[1].reduce(acc => { var t = {}; for (var t of [shared]) {} return Object.assign(t, acc); }, {});",
            false,
        ),
    ] {
        check_file_spelling(
            &mut missing,
            label,
            "no-reduce-accumulator-copy",
            ("case.cjs", source),
            report.then(|| source.find("Object.assign").unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn direct_eval_can_replace_mutable_reducer_targets() {
    let mut missing = Vec::new();
    for (label, file, source, report) in [
        (
            "S97 locally bound direct eval replaces a var",
            "case.cjs",
            "const shared = {}; [1].reduce(acc => { var t = {}; var t; const eval = globalThis.eval; eval('t = shared'); return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S97 direct eval in a nested function sees an enclosing let",
            "case.cjs",
            "const shared = {}; [1].reduce(acc => { let t = {}; (() => { const eval = globalThis.eval; (eval)('t = shared'); })(); return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S97 direct eval cannot replace a const",
            "case.cjs",
            "const shared = {}; [1].reduce(acc => { const t = {}; const eval = globalThis.eval; eval('try { t = shared } catch {}'); return Object.assign(t, acc); }, {});",
            true,
        ),
        (
            "S97 direct eval in an unrelated function",
            "case.cjs",
            "function other() { const eval = globalThis.eval; eval('0'); } [1].reduce(acc => { let t = {}; return Object.assign(t, acc); }, {});",
            true,
        ),
        (
            "S100 optional call to global eval obscures the local let",
            "case.cjs",
            "[1].reduce(acc => { let t = {}; eval?.('t = 1'); return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S97 locally bound optional eval obscures a let",
            "case.cjs",
            "const shared = {}; [1].reduce(acc => { let t = {}; const eval = globalThis.eval; eval?.('t = shared'); return Object.assign(t, acc); }, {});",
            false,
        ),
        (
            "S101 optional eval can become direct in TypeScript emit",
            "case.ts",
            "const shared = {}; [1].reduce(acc => { let t = {}; eval?.('t = shared'); return Object.assign(t, acc); }, {});",
            false,
        ),
    ] {
        check_file_spelling(
            &mut missing,
            label,
            "no-reduce-accumulator-copy",
            (file, source),
            report.then(|| source.find("Object.assign").unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn member_writes_hide_reducer_callbacks_and_targets() {
    let mut missing = Vec::new();
    for (label, rule, source) in [
        (
            "overwritten reducer callback",
            "no-reduce-accumulator-copy",
            "const arr = [1]; const h={cb:(acc,x)=>({...acc,x})}; h.cb=other; arr.reduce(h.cb,{});",
        ),
        (
            "reducer callback overwritten through a bracket",
            "no-reduce-accumulator-copy",
            "const arr = [1]; const h={cb:(acc,x)=>({...acc,x})}; h['cb']=other; arr.reduce(h['cb'],{});",
        ),
        (
            "reducer callback overwritten through an alias",
            "no-reduce-accumulator-copy",
            "const arr = [1]; const h={cb:(acc,x)=>({...acc,x})}; const alias=h; alias.cb=other; arr.reduce(h.cb,{});",
        ),
        (
            "reducer target written by nested callback",
            "no-reduce-accumulator-copy",
            "const xs = [1]; xs.reduce((acc,x)=>{const o={t:{}}; x.forEach(()=>{o.t=acc;}); return Object.assign(o.t,acc);},{});",
        ),
        (
            "reducer target aliased in nested callback",
            "no-reduce-accumulator-copy",
            "const xs = [1]; xs.reduce((acc,x)=>{const o={t:{}}; const alias=o; x.forEach(()=>{alias.t=acc;}); return Object.assign(o.t,acc);},{});",
        ),
        (
            "reducer target written with a bracket in nested callback",
            "no-reduce-accumulator-copy",
            "const xs = [1]; xs.reduce((acc,x)=>{const o={t:{}}; x.forEach(()=>{o['t']=acc;}); return Object.assign(o['t'],acc);},{});",
        ),
    ] {
        check_spelling(&mut missing, label, rule, source, None);
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn with_anywhere_disables_reducer_reports() {
    let mut missing = Vec::new();
    for (label, source) in [
        (
            "unrelated with disables inline object targets",
            "with (o) {} [1].reduce(acc => Object.assign({}, acc), {});",
        ),
        (
            "unrelated with disables spread argument aliases",
            "with (o) {} [1].reduce(acc => { const args = [{}, acc]; return Object.assign(...args); }, {});",
        ),
        (
            "unrelated with disables local target evidence",
            "with (o) {} [1].reduce(acc => { const t = {}; return Object.assign(t, acc); }, {});",
        ),
        (
            "unrelated with disables member target evidence",
            "with (o) {} [1].reduce(acc => { const box = { t: {} }; return Object.assign(box.t, acc); }, {});",
        ),
        (
            "unrelated with disables reducer receiver evidence",
            "with (o) {} const rows = [1]; rows.reduce(acc => Object.assign({}, acc), {});",
        ),
        (
            "with can shadow a named callback",
            "const f = (acc, x) => [...acc, x]; with (o) { [1].reduce(f, []); }",
        ),
        (
            "with can shadow Object",
            "with (o) { [1].reduce((a) => Object.assign({}, a), {}); }",
        ),
        (
            "with can shadow the accumulator",
            "[1].reduce((acc, x) => { with (o) { return [...acc]; } }, [])",
        ),
    ] {
        check_file_spelling(
            &mut missing,
            label,
            "no-reduce-accumulator-copy",
            ("case.cjs", source),
            None,
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn a_spread_consumed_as_the_receiver_does_not_hide_assign_sources() {
    let mut missing = Vec::new();
    for (source, marker) in [
        (
            "const rows = [1]; rows.reduce(acc => Object.assign.call(...[Object], {}, acc), {});",
            Some("Object.assign"),
        ),
        (
            "const rows = [1]; rows.reduce(acc => Object.assign.call(...[Object, {}], acc), {});",
            None,
        ),
        (
            "const rows = [1]; rows.reduce(acc => Object.assign({}, ...[], acc), {});",
            None,
        ),
    ] {
        check_spelling(
            &mut missing,
            source,
            "no-reduce-accumulator-copy",
            source,
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn indirect_eval_and_alias_cycles_leave_incoming_accumulator_reads_visible() {
    let mut missing = Vec::new();
    for (source, marker) in [
        (
            "const rows = [1]; rows.reduce(acc => { let next = {}; (0, eval)('next = shared'); return Object.assign(next, acc); }, {});",
            "Object.assign",
        ),
        (
            "const rows = [1]; rows.reduce(acc => { let next = {}; globalThis.eval('next = shared'); return Object.assign(next, acc); }, {});",
            "Object.assign",
        ),
        (
            "const rows = [1]; rows.reduce(acc => { const a = b; const b = a; return [...a, ...b, ...acc]; }, []);",
            "...acc",
        ),
    ] {
        check_spelling(
            &mut missing,
            source,
            "no-reduce-accumulator-copy",
            source,
            source.find(marker),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn reducer_review_regressions() {
    let mut missing = Vec::new();
    for (label, source, marker) in [
        (
            "R1 custom reduce method",
            "const rows = { reduce(_step, seed) { return seed; } }; rows.reduce(acc => [...acc], []);",
            None,
        ),
        (
            "R1 unproven receiver",
            "rows.reduce(acc => [...acc], []);",
            None,
        ),
        (
            "R1 custom method borrowed by an array",
            "const rows = { reduce(_step, seed) { return seed; } }; rows.reduce.call([1], acc => [...acc], []);",
            None,
        ),
        (
            "R1 native prototype method borrowed by an array",
            "Array.prototype.reduce.call([1], acc => [...acc], []);",
            Some("...acc"),
        ),
        (
            "R2 suspension in an assign argument",
            "[1].reduce(async acc => Object.assign({}, await ready, acc), {});",
            None,
        ),
        (
            "R2 suspension in an argument after the accumulator",
            "[1].reduce(async acc => Object.assign({}, acc, await ready), {});",
            None,
        ),
        (
            "R2 suspension before an immediate function runs",
            "[1].reduce(async acc => (() => [...acc])(await ready), []);",
            None,
        ),
        (
            "R2 suspension before Reflect.apply invokes its target",
            "[1].reduce(async acc => Reflect.apply(() => [...acc], null, [await ready]), []);",
            None,
        ),
        (
            "R2 immediate copy before an unrelated argument suspends",
            "[1].reduce(async acc => consume((() => [...acc])(), await ready), []);",
            Some("...acc"),
        ),
        (
            "R3 parenthesized assignment statement",
            "[1].reduce((acc, n) => { ((acc = [...acc, n])); return acc; }, []);",
            Some("...acc"),
        ),
        (
            "R4 computed string target",
            "[1].reduce(acc => { const box = { ['next']: {} }; return Object.assign(box.next, acc); }, {});",
            Some("Object.assign"),
        ),
        (
            "R4 computed template target",
            "[1].reduce(acc => { const box = { [`next`]: {} }; return Object.assign(box.next, acc); }, {});",
            Some("Object.assign"),
        ),
        (
            "R4 computed target written by a nested callback",
            "[1].reduce(acc => { const box = { ['next']: {} }; defer(() => { box.next = shared; }); return Object.assign(box.next, acc); }, {});",
            None,
        ),
        (
            "R5 Reflect.apply without an argument array",
            "[1].reduce(acc => Reflect.apply(() => [...acc], null, null), []);",
            None,
        ),
        (
            "R5 Reflect.apply with a named argument array",
            "[1].reduce(acc => { const args = []; return Reflect.apply(() => [...acc], null, args); }, []);",
            None,
        ),
        (
            "R7 trailing reduce argument",
            "[1, 2].reduce((acc, n) => [...acc, n], [], null);",
            Some("...acc"),
        ),
    ] {
        check_spelling(
            &mut missing,
            label,
            "no-reduce-accumulator-copy",
            source,
            marker.map(|marker| source.find(marker).unwrap()),
        );
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}
