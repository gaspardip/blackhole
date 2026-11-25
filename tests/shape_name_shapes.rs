use std::{fmt::Write as _, iter};

use blackhole_check::engine::{registry, run};

const RULE: &str = "ts/no-shape-in-symbol-names";
const TS: &str = "case.ts";
const TSX: &str = "case.tsx";
const CJS: &str = "case.cjs";
const JS: &str = "case.js";
const SPELLINGS: [&str; 3] = ["usershapeInput", "userShapeInput", "userSHAPEInput"];
const CONTEXTS: [&str; 9] = [
    "consume({ [KEY]: 0 });",
    "consume({ [KEY]() {} });",
    "consume({ get [KEY]() { return 0; } });",
    "consume({ set [KEY](value) {} });",
    "class C { [KEY] = 0; }",
    "class C { accessor [KEY] = 0; }",
    "class C { [KEY]() {} }",
    "class C { get [KEY]() { return 0; } }",
    "class C { set [KEY](value: number) {} }",
];
const WRAPPERS: [&str; 7] = [
    "^VALUE",
    "(^VALUE)",
    "(^VALUE as unknown)",
    "(^VALUE)!",
    "(^VALUE satisfies unknown)",
    "<unknown>(^VALUE)",
    "(^effect(), VALUE)",
];
const SELECTORS: [(&str, &str); 6] = [
    ("", "'key'"),
    ("", "`key`"),
    ("const field = 'key'; ", "field"),
    ("const field = `key`; ", "field"),
    ("const field = ('key' as string); ", "field"),
    ("", "0"),
];

fn check(failures: &mut Vec<String>, path: &str, marked: &str) {
    check_with_scripts(failures, path, marked, []);
}

fn check_with_scripts<const N: usize>(
    failures: &mut Vec<String>,
    path: &str,
    marked: &str,
    scripts: [(&str, &str); N],
) {
    let mut expected = Vec::new();
    let mut source = String::new();
    for (index, part) in marked.split('^').enumerate() {
        if index > 0 {
            let line = source.split('\n').count();
            let col = source.rsplit('\n').next().unwrap().len() + 1;
            expected.push((path, RULE, line, col));
        }
        source.push_str(part);
    }
    let files = iter::once((path, source.as_bytes()))
        .chain(scripts.map(|(name, script)| (name, script.as_bytes())));
    let findings = run(registry(), files);
    let actual: Vec<_> = findings
        .iter()
        .filter(|hit| hit.rule == RULE || hit.rule == "core/unparseable")
        .map(|hit| (hit.path.as_str(), hit.rule, hit.line, hit.col))
        .collect();
    if actual != expected {
        failures.push(format!(
            "{path}: expected {expected:?}, got {actual:?}\n{source}\n{scripts:?}"
        ));
    }
}

#[test]
fn declared_and_referenced_names_report_at_each_position() {
    let mut failures = Vec::new();
    for spelling in SPELLINGS {
        for (path, template) in [
            (TS, "const ^NAME = 0;"),
            (TS, "function ^NAME() {}"),
            (TS, "const { ^NAME } = source;"),
            (TS, "consume(^NAME);"),
            (TSX, "consume(<^NAME />);"),
            (TSX, "consume(<^NAME.Item />);"),
            (TS, "type T = ^NAME;"),
            (TS, "type ^NAME = string;"),
            (TS, "^NAME: while (ready) {}"),
            (TS, "^NAME: for (;;) { break ^NAME; }"),
            (TS, "^NAME: while (ready) { continue ^NAME; }"),
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
            (TS, "enum E { [^`NAME`] = 0 }"),
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
        let addition = format!("'{}' + '{}'", &spelling[..7], &spelling[7..]);
        let wrapped = format!("('{spelling}' as string)");
        let mut forms = vec![
            (String::new(), format!("'{spelling}'")),
            (String::new(), format!("`{spelling}`")),
            (String::new(), format!("'user' + '{}'", &spelling[4..])),
            (String::new(), addition.clone()),
            (format!("const field = '{spelling}';"), "field".into()),
            (format!("const field = {addition};"), "field".into()),
            (format!("const field = {wrapped};"), "field".into()),
            (format!("const o = {{ key: {addition} }};"), "o.key".into()),
            (format!("const o = {{ key: {wrapped} }};"), "o.key".into()),
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
fn literal_owner_fields_and_extra_reads_report() {
    let mut failures = Vec::new();
    for (declaration, field) in [
        ("", "'key': 'userShapeInput'"),
        ("", "['key']: 'userShapeInput'"),
        ("", "[`key`]: 'userShapeInput'"),
        ("const key = 'userShapeInput';", "key"),
    ] {
        for read in [
            "",
            "consume(o.other);",
            "consume(o['other']);",
            "consume(<o.Other />);",
        ] {
            let source = format!(
                "{declaration} const o = {{ {field}, other() {{}} }};\n                 {read}\nconsume({{ [^o.key]: 0 }});"
            );
            check(&mut failures, TSX, &source);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn broken_owner_preconditions_stay_silent() {
    let mut failures = Vec::new();
    for (valid, invalid) in [
        ("other: extra", "...extra"),
        ("other() { return 0; }", "get other() { return 0; }"),
        ("other(value) {}", "set other(value) {}"),
        ("other: null", "__proto__: null"),
        ("['other']: 0", "[field]: 0"),
        ("[`other`]: 0", "[`oth${'er'}`]: 0"),
        ("1: 0", "[1]: 0"),
        ("1: 0", "1n: 0"),
        ("'1.1': 0", "1.1: 0"),
        ("'10': 0", "1e1: 0"),
        ("1000: 0", "1_000: 0"),
        ("16: 0", "0x10: 0"),
        ("'9007199254740992': 0", "9007199254740992: 0"),
    ] {
        for (extra_field, mark) in [(valid, "^"), (invalid, "")] {
            check(
                &mut failures,
                TS,
                &format!(
                    "const field = 'other'; const o = {{ key: 'userShapeInput', {extra_field} }};\n                     consume({{ [{mark}o.key]: 0 }});"
                ),
            );
        }
    }
    for (valid, invalid) in [
        ("o.other;", "o.other = 1;"),
        ("o.other;", "delete o.other;"),
        ("consume(o.other);", "consume(o);"),
        ("const alias = o.other;", "const alias = o;"),
        ("o.other;", "o.other();"),
        ("type T = string;", "type T = typeof o.key;"),
        ("interface O {}", "interface o {}"),
        ("type Evaluate = string;", "type eval = string;"),
    ] {
        for (other_use, mark) in [(valid, "^"), (invalid, "")] {
            check(
                &mut failures,
                TS,
                &format!(
                    "const o = {{ key: 'userShapeInput', other() {{}} }};\n                     {other_use}\nconsume({{ [{mark}o.key]: 0 }});"
                ),
            );
        }
    }
    for (declaration, export, mark) in [
        (
            "const o = { key: 'userShapeInput' };",
            "export type { o };",
            "^",
        ),
        ("const o = { key: 'userShapeInput' };", "export { o };", ""),
        ("export const o = { key: 'userShapeInput' };", "", ""),
        ("let o = { key: 'userShapeInput' };", "", ""),
        ("const { o } = { o: { key: 'userShapeInput' } };", "", ""),
        ("const o = { key: 'userShapeInput' };", "", "^"),
    ] {
        check(
            &mut failures,
            TS,
            &format!("{declaration}\n{export}\nconsume({{ [{mark}o.key]: 0 }});"),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn broken_alias_and_expression_preconditions_stay_silent() {
    let mut failures = Vec::new();
    for (valid, invalid) in [
        (
            "const key = 'userShapeInput'",
            "const { key } = { key: 'userShapeInput' }",
        ),
        ("const key = 'userShapeInput'", "let key = 'userShapeInput'"),
        (
            "type Key = string; const key = 'userShapeInput'",
            "type key = string; const key = 'userShapeInput'",
        ),
        (
            "type Evaluate = string; const key = 'userShapeInput'",
            "type eval = string; const key = 'userShapeInput'",
        ),
    ] {
        for (declaration, mark) in [(valid, "^"), (invalid, "")] {
            check(
                &mut failures,
                TS,
                &format!("{declaration};\nconsume({{ [{mark}key]: 0 }});"),
            );
        }
    }
    for (setup, valid, invalid) in [
        ("", "'userShapeInput' + '1'", "'userShapeInput' + 1"),
        ("", "`userShapeInput`", "`userSha${'peInput'}`"),
        ("", "'userSha' + 'peInput'", "'userSha'.concat('peInput')"),
        (
            "enum E { Key = 'userShapeInput' }",
            "'userShapeInput'",
            "E.Key",
        ),
    ] {
        for (expression, mark) in [(valid, "^"), (invalid, "")] {
            check(
                &mut failures,
                TS,
                &format!("{setup}\nconsume({{ [{mark}{expression}]: 0 }});"),
            );
        }
    }
    for (setup, expression) in [
        ("const key = 'userShapeInput';", "key"),
        ("const o = { key: 'userShapeInput' };", "o.key"),
    ] {
        for (scope, mark) in [("", "^"), ("with (scope)", "")] {
            check(
                &mut failures,
                CJS,
                &format!("{setup} {scope} {{ consume({{ [{mark}{expression}]: 0 }}); }}"),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn broken_selector_preconditions_stay_silent() {
    let mut failures = Vec::new();
    for (setup, control, selector) in [
        ("const a = 'key'; const b = a;", "a", "b"),
        ("const i = 0;", "0", "i"),
        ("", "'key'", "'ke' + 'y'"),
        ("const fields = { key: 'key' };", "'key'", "fields.key"),
        ("", "`key`", "`ke${'y'}`"),
        ("", "0", "0x0"),
    ] {
        for (read, mark) in [(control, "^"), (selector, "")] {
            check(
                &mut failures,
                TS,
                &format!(
                    "const o = {{ key: 'userShapeInput', 0: 'userShapeInput' }};\n                     {setup}\nconsume({{ [{mark}o[{read}]]: 0 }});"
                ),
            );
        }
    }
    for (declaration, mark) in [
        ("const field = 'key'", "^"),
        ("const { field } = { field: 'key' }", ""),
    ] {
        check(
            &mut failures,
            TS,
            &format!(
                "const o = {{ key: 'userShapeInput' }}; {declaration};\n                 consume({{ [{mark}o[field]]: 0 }});"
            ),
        );
    }
    for (key, selector) in [
        ("1.1", "1.1"),
        ("10", "1e1"),
        ("1000", "1_000"),
        ("1", "+1"),
        ("-1", "-1"),
        ("9007199254740992", "9007199254740992"),
    ] {
        for (read, mark) in [(format!("'{key}'"), "^"), (selector.into(), "")] {
            check(
                &mut failures,
                TS,
                &format!(
                    "const o = {{ '{key}': 'userShapeInput' }}; consume({{ [{mark}o[{read}]]: 0 }});"
                ),
            );
        }
    }
    for (selector, mark) in [("1", "^"), ("1n", "")] {
        check(
            &mut failures,
            TS,
            &format!(
                "const o = {{ 1: 'userShapeInput' }}; consume({{ [{mark}o[{selector}]]: 0 }});"
            ),
        );
    }
    check(
        &mut failures,
        TS,
        "const o = { 9007199254740991: 'userShapeInput' }; consume({ [^o[9007199254740991]]: 0 });",
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn cyclic_dynamic_and_over_budget_additions_stay_silent() {
    let mut failures = Vec::new();
    for (seed, mark) in [("''", "^"), ("b0 + ''", ""), ("getPrefix()", "")] {
        let mut source = format!("const b0 = {seed};\n");
        for i in 1..=3 {
            writeln!(source, "const b{i} = b{} + '';", i - 1).unwrap();
        }
        writeln!(source, "consume({{ [{mark}b3 + 'userShapeInput']: 0 }});").unwrap();
        check(&mut failures, TS, &source);
    }

    let spelling = "userShapeInput";
    let doublings = 14;
    let folded_len = (1 << doublings) + spelling.len();
    let mut declarations = "const b0 = 'x';\n".to_owned();
    for i in 1..=doublings {
        let prev = i - 1;
        writeln!(declarations, "const b{i} = b{prev} + b{prev};").unwrap();
    }
    for (budget, mark) in [(folded_len, "^"), (folded_len - 1, "")] {
        let mut source =
            format!("{declarations}consume({{ [{mark}b{doublings} + '{spelling}']: 0 }});");
        source.extend(iter::repeat_n(' ', budget + mark.len() - source.len()));
        check(&mut failures, TS, &source);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn dynamic_and_cross_script_writes_still_report() {
    let mut failures = Vec::new();
    for (path, effect) in [
        (JS, r#"Function("o.key = 'plain'")();"#),
        (JS, r#"new Function("o.key = 'plain'")();"#),
        (TSX, r#"consume(<X attr="ev&#x61;l" />);"#),
        (
            JS,
            "Object.defineProperty(Object.prototype, 'other', { get() { this.key = 'plain'; } }); o.other;",
        ),
    ] {
        check(
            &mut failures,
            path,
            &format!(
                "const o = {{ key: 'userShapeInput' }};\n{effect}\nconsume({{ [^o.key]: 0 }});"
            ),
        );
    }
    check(
        &mut failures,
        JS,
        r#"const o = { key: 'userShapeInput' };
setTimeout("o.key = 'plain'; setTimeout(read, 0)", 0);
function read() { consume({ [^o.key]: 0 }); }"#,
    );
    check_with_scripts(
        &mut failures,
        JS,
        "const o = { key: 'userShapeInput' }; function read() { consume({ [^o.key]: 0 }); }",
        [("writer.js", "o.key = 'plain'; read();")],
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn external_names_and_string_values_stay_silent() {
    let mut failures = Vec::new();
    for (path, source) in [
        (TS, "const value = 'userShapeInput';"),
        (TS, "import data from './userShapeInput.json';"),
        (
            TS,
            "import data from './data.json' with { type: 'userShapeInput' };",
        ),
        (TS, "declare module 'userShapeInput' {}"),
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
