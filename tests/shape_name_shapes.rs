use blackhole_check::engine::{registry, run};

const RULE: &str = "ts/no-shape-in-symbol-names";
const TS: &str = "case.ts";
const TSX: &str = "case.tsx";
const CJS: &str = "case.cjs";
const SPELLINGS: [&str; 3] = ["usershapeInput", "userShapeInput", "userSHAPEInput"];
const CONTEXTS: [&str; 2] = ["consume({ [KEY]: 0 });", "class C { [KEY] = 0; }"];
const WRAPPERS: [&str; 7] = [
    "^VALUE",
    "(^VALUE)",
    "(^VALUE as unknown)",
    "(^VALUE)!",
    "(^VALUE satisfies unknown)",
    "<unknown>(^VALUE)",
    "(^effect(), VALUE)",
];
const SELECTORS: [(&str, &str); 5] = [
    ("", "'key'"),
    ("", "`key`"),
    ("const field = 'key'; ", "field"),
    ("const field = `key`; ", "field"),
    ("", "0"),
];

fn check(failures: &mut Vec<String>, path: &str, marked: &str) {
    let expected: Vec<_> = marked
        .split_once('^')
        .map(|(prefix, _)| {
            (
                RULE,
                prefix.split('\n').count(),
                prefix.rsplit('\n').next().unwrap().len() + 1,
            )
        })
        .into_iter()
        .collect();
    let source = marked.replace('^', "");
    let actual: Vec<_> = run(registry(), [(path, source.as_bytes())])
        .into_iter()
        .filter(|hit| hit.rule == RULE || hit.rule == "core/unparseable")
        .map(|hit| (hit.rule, hit.line, hit.col))
        .collect();
    if actual != expected {
        failures.push(format!(
            "{path}: expected {expected:?}, got {actual:?}\n{source}"
        ));
    }
}

#[test]
fn declared_and_referenced_names_report_once() {
    let mut failures = Vec::new();
    for spelling in SPELLINGS {
        for (path, template) in [
            (TS, "const ^NAME = 0;"),
            (TS, "const { ^NAME } = source;"),
            (TS, "consume(^NAME);"),
            (TSX, "consume(<^NAME />);"),
            (TSX, "consume(<^NAME.Item />);"),
            (TS, "type T = ^NAME;"),
            (TS, "type ^NAME = string;"),
            (TS, "^NAME: while (ready) {}"),
            (TS, "class C { ^#NAME = 0; }"),
            (TS, "consume({ ^NAME: 0 });"),
            (TS, "consume({ ^'NAME': 0 });"),
            (TS, "consume({ ^NAME() {} });"),
            (TS, "consume({ ^'NAME'() {} });"),
            (TS, "consume({ get ^NAME() { return 0; } });"),
            (TS, "consume({ get ^'NAME'() { return 0; } });"),
            (TS, "consume({ set ^NAME(value) {} });"),
            (TS, "consume({ set ^'NAME'(value) {} });"),
            (TS, "class C { ^NAME = 0; }"),
            (TS, "class C { ^'NAME' = 0; }"),
            (TS, "class C { ^NAME() {} }"),
            (TS, "class C { ^'NAME'() {} }"),
            (TS, "class C { get ^NAME() { return 0; } }"),
            (TS, "class C { get ^'NAME'() { return 0; } }"),
            (TS, "class C { set ^NAME(value: number) {} }"),
            (TS, "class C { set ^'NAME'(value: number) {} }"),
            (TS, "class C { accessor ^NAME = 0; }"),
            (TS, "class C { accessor ^'NAME' = 0; }"),
            (TS, "interface I { ^NAME: number; }"),
            (TS, "interface I { ^'NAME': number; }"),
            (TS, "interface I { ^NAME(): void; }"),
            (TS, "interface I { ^'NAME'(): void; }"),
            (TS, "interface I { get ^NAME(): number; }"),
            (TS, "interface I { get ^'NAME'(): number; }"),
            (TS, "interface I { set ^NAME(value: number); }"),
            (TS, "interface I { set ^'NAME'(value: number); }"),
            (TS, "type T = { ^NAME: number; };"),
            (TS, "type T = { ^'NAME': number; };"),
            (TS, "type T = { ^NAME(): void; };"),
            (TS, "type T = { ^'NAME'(): void; };"),
            (TS, "type T = { get ^NAME(): number; };"),
            (TS, "type T = { get ^'NAME'(): number; };"),
            (TS, "type T = { set ^NAME(value: number); };"),
            (TS, "type T = { set ^'NAME'(value: number); };"),
            (TS, "enum E { ^NAME = 0 }"),
            (TS, "enum E { ^'NAME' = 0 }"),
            (TS, "import { ^NAME as local } from 'm';"),
            (TS, "import { ^'NAME' as local } from 'm';"),
            (TS, "export { ^NAME as remote } from 'm';"),
            (TS, "export { ^'NAME' as remote } from 'm';"),
            (TS, "export { local as ^NAME } from 'm';"),
            (TS, "export { local as ^'NAME' } from 'm';"),
            (TS, "export * as ^NAME from 'm';"),
            (TS, "export * as ^'NAME' from 'm';"),
            (TS, "import data from 'm' with { ^NAME: 'json' };"),
            (TS, "import data from 'm' with { ^'NAME': 'json' };"),
        ] {
            check(&mut failures, path, &template.replace("NAME", spelling));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn known_strings_report_through_each_wrapper() {
    let mut failures = Vec::new();
    for spelling in SPELLINGS {
        let owner = format!("const o = {{ key: '{spelling}', 0: '{spelling}' }};");
        let mut forms = vec![
            (String::new(), format!("'{spelling}'")),
            (String::new(), format!("`{spelling}`")),
            (String::new(), format!("'user' + '{}'", &spelling[4..])),
            (format!("const field = '{spelling}';"), "field".into()),
            (owner.clone(), "o.key".into()),
            (owner.clone(), "o?.key".into()),
        ];
        for (declaration, selector) in SELECTORS {
            for access in ["o[SELECTOR]", "o?.[SELECTOR]"] {
                forms.push((
                    format!("{declaration}{owner}"),
                    access.replace("SELECTOR", selector),
                ));
            }
        }
        for path in [TS, TSX] {
            for context in CONTEXTS {
                for wrapper in WRAPPERS {
                    if path == TSX && wrapper.starts_with('<') {
                        continue;
                    }
                    for (prelude, expression) in &forms {
                        let key = wrapper.replace("VALUE", expression);
                        let source = format!("{prelude}\n{}", context.replace("KEY", &key));
                        check(&mut failures, path, &source);
                    }
                }
            }
        }
        for context in CONTEXTS {
            for (declaration, selector) in SELECTORS {
                for wrapper in WRAPPERS {
                    let selector = wrapper.replace('^', "").replace("VALUE", selector);
                    let body = context.replace("KEY", &format!("^o[{selector}]"));
                    check(&mut failures, TS, &format!("{declaration}{owner}\n{body}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn broken_known_string_preconditions_stay_silent() {
    let mut failures = Vec::new();
    for extra_field in [
        "...extra",
        "get other() { return 0; }",
        "set other(value) {}",
        "__proto__: null",
        "[field]: 0",
        "[1]: 0",
        "1.1: 0",
        "1e1: 0",
        "1_000: 0",
        "0x10: 0",
        "9007199254740992: 0",
    ] {
        check(
            &mut failures,
            TS,
            &format!(
                "const field = 'other'; const o = {{ key: 'userShapeInput', {extra_field} }};\n\
                 consume({{ [o.key]: 0 }});"
            ),
        );
    }
    for other_use in [
        "export { o };",
        "o.other = 1;",
        "delete o.other;",
        "consume(o);",
        "const alias = o;",
        "o.other();",
        "type T = typeof o.key;",
        "interface o {}",
        "type eval = string;",
    ] {
        check(
            &mut failures,
            TS,
            &format!(
                "const o = {{ key: 'userShapeInput', other() {{}} }};\n\
                 {other_use}\nconsume({{ [o.key]: 0 }});"
            ),
        );
    }
    for (path, source) in [
        (
            TS,
            "const { key } = { key: 'userShapeInput' };\nconsume({ [key]: 0 });",
        ),
        (
            TS,
            "type key = string; const key = 'userShapeInput';\nconsume({ [key]: 0 });",
        ),
        (TS, "consume({ ['userShapeInput' + 1]: 0 });"),
        (TS, "consume({ [`userSha${'peInput'}`]: 0 });"),
        (TS, "consume({ ['userSha'.concat('peInput')]: 0 });"),
        (
            TS,
            "enum E { Key = 'userShapeInput' }\nconsume({ [E.Key]: 0 });",
        ),
        (
            CJS,
            "const key = 'userShapeInput'; with (scope) { consume({ [key]: 0 }); }",
        ),
        (
            CJS,
            "const o = { key: 'userShapeInput' }; with (scope) { consume({ [o.key]: 0 }); }",
        ),
        (
            TS,
            "type eval = string; const key = 'userShapeInput';\nconsume({ [key]: 0 });",
        ),
    ] {
        check(&mut failures, path, source);
    }
    for (setup, selector) in [
        ("const a = 'key'; const b = a;", "b"),
        ("const i = 0;", "i"),
        ("", "'ke' + 'y'"),
        ("const fields = { key: 'key' };", "fields.key"),
        ("", "`ke${'y'}`"),
        ("", "0x0"),
    ] {
        check(
            &mut failures,
            TS,
            &format!(
                "const o = {{ key: 'userShapeInput', 0: 'userShapeInput' }};\n\
                 {setup}\nconsume({{ [o[{selector}]]: 0 }});"
            ),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn external_names_and_string_values_stay_silent() {
    let mut failures = Vec::new();
    for (path, source) in [
        (TS, "const value = 'userShapeInput';"),
        (TSX, "consume(<Area userShapeInput='rect' />);"),
        (TSX, "consume(<svg:userShapeInput />);"),
        (TSX, "consume(<userShapeInput:path />);"),
        (TSX, "consume(<svg:path xlink:userShapeInput='rect' />);"),
        (TSX, "consume(<svg:path userShapeInput:href='rect' />);"),
        (TSX, "consume(<Root.UserShapeInput />);"),
        (TS, "consume(a.userShapeInput);"),
        (TS, "const { userShapeInput: local } = external;"),
        (TS, "({ userShapeInput: local } = external);"),
        (TS, "type T = typeof root.userShapeInput;"),
    ] {
        check(&mut failures, path, source);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
