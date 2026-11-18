#[path = "common/run_rule.rs"]
mod single;

type Question = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);
const QUESTIONS: &[Question] = &[
    (
        "no-unknown-parameters",
        "function f(x: POS) {}",
        "@string | TYPE",
        &["unknown"],
        &["number"],
    ),
    (
        "no-object-parameters",
        "function f(x: POS) {}",
        "@string | TYPE",
        &["object"],
        &["number"],
    ),
    (
        "no-unknown-type-aliases",
        "type @Result = string | POS;",
        "TYPE",
        &["unknown"],
        &["number"],
    ),
    (
        "no-unknown-returns",
        "declare function f(): POS;",
        "@Promise<TYPE>",
        &["unknown"],
        &["number"],
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = POS;",
        "@Record<string, TYPE>",
        &["unknown"],
        &["number", "unknown[]", "[unknown]"],
    ),
    (
        "no-known-value-widening",
        "declare function f(x: POS): x is string; f(@'x');",
        "TYPE",
        &["unknown"],
        &["string"],
    ),
    (
        "no-known-value-widening",
        "const value: POS = @{ id: 1 };",
        "Record<TYPE, number>",
        &["string"],
        &["'id'"],
    ),
    (
        "no-widen-then-assert",
        "const a: POS = { id: 1 }; const b = @a as { id: number };",
        "Record<TYPE, unknown>",
        &["string"],
        &["'id'"],
    ),
    (
        "no-widen-then-assert",
        "const a: POS = { id: 1 }; const b = @a as { id: number };",
        "Record<string, TYPE>",
        &["unknown"],
        &["number"],
    ),
    (
        "no-widen-then-assert",
        "const a: POS = { id: 1 }; const b = @a as { id: number };",
        "Readonly<TYPE>",
        &["Record<string, unknown>"],
        &["{ id: number }"],
    ),
    (
        "no-widen-then-assert",
        "const a: object = { id: 1 }; const b = @a as POS;",
        "TYPE & { name: string }",
        &["{ id: number }"],
        &["number"],
    ),
    (
        "no-widen-then-assert",
        "const a: Record<string, unknown> = { id: 1 }; const b = @a as POS;",
        "Readonly<Record<string, TYPE>>",
        &["number"],
        &["unknown"],
    ),
    (
        "no-widen-then-assert",
        "const a: POS = { id: 1 }; const b = @a as { id: number };",
        "{ [id: TYPE]: unknown }",
        &["string"],
        &["`key${string}`"],
    ),
    (
        "no-widen-then-assert",
        "const a: POS = { id: 1 }; const b = @a as { id: number };",
        "{ [id: string]: TYPE }",
        &["unknown"],
        &["number"],
    ),
    (
        "no-widen-then-assert",
        "const a: POS = { id: 1 }; const b = @a as { id: number };",
        "Record<number | TYPE, unknown>",
        &["string"],
        &["'id'"],
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = POS;",
        "@Record<string, { id?: TYPE }>",
        &["never"],
        &["number"],
    ),
    (
        "no-known-value-widening",
        "declare function f(x: unknown): x is string; declare const arg: POS; f(@arg);",
        "TYPE",
        &["string"],
        &["unknown"],
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = POS;",
        "@Record<string, TYPE & {}>",
        &["unknown"],
        &["number"],
    ),
    (
        "no-widen-then-assert",
        "const a: POS = { id: 1 }; const b = @a as [number];",
        "TYPE",
        &["object"],
        &["number"],
    ),
    (
        "no-widen-then-assert",
        "const a: POS = 1; const b = @a as number;",
        "TYPE",
        &["unknown"],
        &["number"],
    ),
    (
        "no-widen-then-assert",
        "const a: object = []; const b = @a as POS;",
        "TYPE",
        &["string[]", "[string]"],
        &["string"],
    ),
    (
        "no-known-value-widening",
        "const v: POS = @1;",
        "TYPE",
        &["unknown"],
        &["number"],
    ),
    (
        "no-widen-then-assert",
        "declare const source: POS; const wide: Record<string, unknown> = source; const picked = @wide as { id: number };",
        "Record<TYPE, unknown>",
        &["'id'"],
        &["string"],
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = @Record<string, POS>;",
        "{ [K in string]: TYPE }",
        &[],
        &["number"],
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = @Record<string, POS>;",
        "Record<string, TYPE>",
        &[],
        &["number"],
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = @Record<string, POS>;",
        "TYPE[]",
        &[],
        &["unknown"],
    ),
    (
        "no-unsafe-dictionary-type",
        "type Result = @Record<string, POS>;",
        "[TYPE]",
        &[],
        &["unknown"],
    ),
    (
        "no-widen-then-assert",
        "const a: object = []; const p = @a as POS;",
        "readonly TYPE[]",
        &["number"],
        &[],
    ),
    (
        "no-unknown-parameters",
        "function f(x: POS) {}",
        "@readonly [TYPE]",
        &[],
        &["unknown"],
    ),
    (
        "no-unsafe-dictionary-type",
        "interface E {} interface F { id: number } interface S extends POS {} type D = @Record<string, S>;",
        "TYPE",
        &["E"],
        &["F"],
    ),
];

fn cell(
    rule: &str,
    template: &str,
    position: &str,
    spelling: &str,
    pre: &str,
    post: &str,
) -> (Vec<usize>, usize) {
    let line = template.replace("POS", &position.replace("TYPE", spelling));
    let col = line.find('@').unwrap() + 1;
    let source = format!("{pre}\n{}\n{post}", line.replace('@', ""));
    assert!(
        single::run_rule("core/unparseable", "types.ts", &source).is_empty(),
        "{source}"
    );
    let hits = single::run_rule(&format!("ts/{rule}"), "types.ts", &source)
        .into_iter()
        .filter(|h| h.line == 2)
        .map(|h| h.col)
        .collect();
    (hits, col)
}

#[test]
fn semantic_questions_follow_type_spellings() {
    let mut differences = Vec::new();
    for &(rule, template, position, rejects, accepts) in QUESTIONS {
        for (ty, rejects) in rejects
            .iter()
            .map(|t| (*t, true))
            .chain(accepts.iter().map(|t| (*t, false)))
        {
            let (direct, at) = cell(rule, template, position, ty, "", "");
            assert_eq!(
                direct,
                if rejects { vec![at] } else { vec![] },
                "{template} {position} {ty}"
            );
            for (column, pre, name, post) in [
                ("plain", "type X = TY;", "X", ""),
                ("chain", "type Y = TY; type X = Y;", "X", ""),
                ("forward", "", "X", "type X = TY;"),
                ("namesake", "const X = 1; type X = TY;", "X", ""),
                ("shadow", "type X = never; { type X = TY;", "X", "}"),
                ("application", "type X<T> = T;", "X<TY>", ""),
                ("default", "type X<T = TY> = T;", "X", ""),
                (
                    "named application",
                    "type Y<T> = T; type X = Y<TY>;",
                    "X",
                    "",
                ),
                ("forwarder", "type Y<T> = T; type X<T> = Y<T>;", "X<TY>", ""),
            ] {
                let (hits, col) = cell(
                    rule,
                    template,
                    position,
                    &name.replace("TY", ty),
                    &pre.replace("TY", ty),
                    &post.replace("TY", ty),
                );
                let expected: Vec<_> = direct.iter().map(|_| col).collect();
                if hits != expected {
                    differences.push(format!(
                        "{rule} {template} {position} {ty} {column}: {hits:?} expected {expected:?}"
                    ));
                }
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
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

#[test]
fn default_can_name_its_own_instantiation() {
    for spelling in ["A", "A<unknown>"] {
        let source = format!("type A<T = A<unknown>> = T | string;\ntype X = {spelling};");
        let hits = single::run_rule("ts/no-unknown-type-aliases", "types.ts", &source);
        assert_eq!(
            hits.iter()
                .map(|h| (h.line, h.col, h.note.as_deref()))
                .collect::<Vec<_>>(),
            [(2, 6, None)]
        );
    }
}
