#[path = "common/run_rule.rs"]
mod single;

const QUESTIONS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "no-unknown-parameters",
        "function f(x: string | TYPE) {}",
        "unknown",
        "number",
        "string |",
    ),
    (
        "no-object-parameters",
        "function f(x: string | TYPE) {}",
        "object",
        "number",
        "string |",
    ),
    (
        "no-unknown-type-aliases",
        "type Result = string | TYPE;",
        "unknown",
        "number",
        "Result",
    ),
    (
        "no-unknown-returns",
        "declare function f(): Promise<TYPE>;",
        "unknown",
        "number",
        "Promise",
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = Record<string, TYPE>;",
        "unknown",
        "number",
        "Record",
    ),
    (
        "no-known-value-widening",
        "declare function f(x: TYPE): x is string; f('x');",
        "unknown",
        "string",
        "'x'",
    ),
    (
        "no-known-value-widening",
        "const value: Record<TYPE, number> = { id: 1 };",
        "string",
        "'id'",
        "{ id:",
    ),
    (
        "no-widen-then-assert",
        "const a: Record<TYPE, unknown> = { id: 1 }; const b = a as { id: number };",
        "string",
        "'id'",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: Record<string, TYPE> = { id: 1 }; const b = a as { id: number };",
        "unknown",
        "number",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: Readonly<TYPE> = { id: 1 }; const b = a as { id: number };",
        "Record<string, unknown>",
        "{ id: number }",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: object = { id: 1 }; const b = a as TYPE & { name: string };",
        "{ id: number }",
        "number",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: Record<string, unknown> = { id: 1 }; const b = a as Readonly<Record<string, TYPE>>;",
        "number",
        "unknown",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: { [id: TYPE]: unknown } = { id: 1 }; const b = a as { id: number };",
        "string",
        "`key${string}`",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: { [id: string]: TYPE } = { id: 1 }; const b = a as { id: number };",
        "unknown",
        "number",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: Record<number | TYPE, unknown> = { id: 1 }; const b = a as { id: number };",
        "string",
        "'id'",
        "a as",
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = Record<string, { id?: TYPE }>;",
        "never",
        "number",
        "Record",
    ),
    (
        "no-known-value-widening",
        "declare function f(x: unknown): x is string; declare const arg: TYPE; f(arg);",
        "string",
        "unknown",
        "arg);",
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = Record<string, TYPE & {}>;",
        "unknown",
        "number",
        "Record",
    ),
    (
        "no-widen-then-assert",
        "const a: TYPE = { id: 1 }; const b = a as [number];",
        "object",
        "number",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: TYPE = 1; const b = a as number;",
        "unknown",
        "number",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: object = []; const b = a as TYPE;",
        "string[]",
        "string",
        "a as",
    ),
    (
        "no-widen-then-assert",
        "const a: object = []; const b = a as TYPE;",
        "[string]",
        "string",
        "a as",
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = Record<string, TYPE>;",
        "unknown",
        "unknown[]",
        "Record",
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = Record<string, TYPE>;",
        "unknown",
        "[unknown]",
        "Record",
    ),
];

#[test]
fn semantic_questions_follow_type_spellings() {
    let mut differences = Vec::new();
    for &(rule, template, reject, accept, at) in QUESTIONS {
        for (ty, reports) in [(reject, true), (accept, false)] {
            for (column, pre, spelling, post) in [
                ("direct", "", ty, ""),
                ("plain", "type X = TY;", "X", ""),
                ("chain", "type Y = TY; type X = Y;", "X", ""),
                ("forward", "", "X", "type X = TY;"),
                ("namesake", "const X = 1; type X = TY;", "X", ""),
                ("shadow", "type X = never; { type X = TY;", "X", "}"),
            ] {
                let line = template.replace("TYPE", spelling);
                let source = format!(
                    "{}\n{line}\n{}",
                    pre.replace("TY", ty),
                    post.replace("TY", ty)
                );
                assert!(
                    single::run_rule("core/unparseable", "types.ts", &source).is_empty(),
                    "{source}"
                );
                let positions: Vec<_> =
                    single::run_rule(&format!("ts/{rule}"), "types.ts", &source)
                        .into_iter()
                        .filter(|f| f.line == 2)
                        .map(|f| f.col)
                        .collect();
                let expected: Vec<_> = reports
                    .then(|| line.find(at).unwrap() + 1)
                    .into_iter()
                    .collect();
                if positions != expected {
                    differences.push(format!(
                        "{rule} {template} {ty} {column}: {positions:?} expected {expected:?}"
                    ));
                }
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn empty_heritage_and_optional_members() {
    for (declarations, reports) in [
        ("type Base = {}; interface Shape extends Base {}", true),
        (
            "interface Base<T> {} interface Shape extends Base<string> {}",
            true,
        ),
        (
            "type Missing = never; interface Shape { id?: Missing }",
            true,
        ),
        (
            "interface Base<T> { id: T } interface Shape extends Base<string> {}",
            false,
        ),
        ("class Base {} interface Shape extends Base {}", false),
    ] {
        let source = format!("{declarations}\ntype Dict = Record<string, Shape>;");
        let hits = single::run_rule("ts/no-unsafe-dictionary-type", "types.ts", &source);
        assert_eq!(
            hits.iter().any(|h| (h.line, h.col) == (2, 13)),
            reports,
            "{source}"
        );
    }
}

#[test]
fn written_annotations_distinguish_named_shapes() {
    for shape in ["{ id: number }", "{ [K in 'id']: number }"] {
        for (annotation, reports) in [(shape, true), ("Shape", false)] {
            let source = format!("type Shape = {shape};\nconst value: {annotation} = {{ id: 1 }};");
            let hits = single::run_rule("ts/no-known-value-widening", "types.ts", &source);
            assert_eq!(hits.len(), usize::from(reports), "{source}");
        }
    }
}

#[test]
fn generic_defaults_can_reenter_under_a_different_question() {
    let mut missing = Vec::new();
    for body in [
        "Record<string, A<unknown>>",
        "{ [key: string]: A<unknown> }",
        "{ [K in string]: A<unknown> }",
        "Readonly<Record<string, A<unknown>>>",
    ] {
        let source = format!("type A<T = {body}> = T;\ntype Dict = A;");
        let hits = single::run_rule("ts/no-unsafe-dictionary-type", "types.ts", &source);
        if !hits.iter().any(|h| (h.line, h.col) == (2, 13)) {
            missing.push(body);
        }
    }
    assert!(missing.is_empty(), "{missing:?}");
}
