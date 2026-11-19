#[path = "common/run_rule.rs"]
mod single;

fn positions(hits: &[blackhole_check::engine::Finding]) -> Vec<(usize, usize, Option<&str>)> {
    hits.iter()
        .map(|h| (h.line, h.col, h.note.as_deref()))
        .collect()
}

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
    (
        "no-widen-then-assert",
        "interface Model {id:number} interface Other {name:string} declare const original: Model; const wide: object = original; const picked = @wide as POS;",
        "TYPE",
        &["Model"],
        &["Other"],
    ),
    (
        "no-widen-then-assert",
        "interface Model {id:number} interface Other {name:string} declare const original: POS; const wide: object = original; const picked = @wide as Model;",
        "TYPE",
        &["Model"],
        &["Other"],
    ),
    (
        "no-widen-then-assert",
        "interface Model {id:number} declare const original: POS; const wide: object = original; const picked = @wide as POS;",
        "TYPE",
        &["Model"],
        &[],
    ),
    (
        "no-unsafe-dictionary-type",
        "interface Base<T> { id?: T } interface Model extends Base<POS> {} type D = @Record<string, Model>;",
        "TYPE",
        &["never"],
        &["number"],
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
            let inline = template.replace(
                "interface Model extends Base<POS> {}",
                "interface Model { id?: POS }",
            );
            let (direct, at) = cell(rule, &inline, position, ty, "", "");
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
                (
                    "namespace",
                    "namespace N { export type X = TY; }",
                    "N.X",
                    "",
                ),
                (
                    "nested namespace",
                    "namespace N { export namespace M { export type X = TY; } }",
                    "N.M.X",
                    "",
                ),
                (
                    "namespace import",
                    "namespace N { export namespace M { export type X = TY; } } import M = N.M;",
                    "M.X",
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
        assert_eq!(positions(&hits), [(2, 6, None)]);
    }
}

#[test]
fn assertion_back_to_a_declaration_follows_its_binding() {
    for (origin, local, target, expected) in [
        ("Model", "", "Model", true),
        ("Model", "type Alias = Model;", "Alias", true),
        ("Model", "type Id<T> = T;", "Id<Model>", true),
        ("Model", "type Id<T = Model> = T;", "Id", true),
        ("Model & Tag", "type Id<T> = T;", "Id<Model> & Tag", true),
        ("Model", "interface Model { name: string }", "Model", false),
        ("Model", "interface Other { name: string }", "Other", false),
        ("Model", "type Same = Model;", "Same", true),
        ("Model", "interface Other { id: number }", "Other", false),
        ("Model", "", "{ id: number }", true),
    ] {
        let source = format!(
            "interface Model {{ id: number }} interface Tag {{ tag: string }}\ndeclare const original: {origin};\nconst wide: object = original;\n{{ {local}\nconst picked = wide as {target};\n}}"
        );
        let hits = single::run_rule("ts/no-widen-then-assert", "binding.ts", &source);
        assert_eq!(
            positions(&hits),
            expected
                .then_some((5, 16, None))
                .into_iter()
                .collect::<Vec<_>>(),
            "{source}"
        );
    }
}

#[test]
fn qualified_object_source_is_already_broad() {
    for source_type in ["Types.Wide", "object"] {
        let text = format!(
            "namespace Types {{ export type Wide = object; }}\ndeclare const original: {source_type};\nconst wide: object = original;\nconst picked = wide as {{ id: number }};"
        );
        assert!(
            single::run_rule("ts/no-widen-then-assert", "source.ts", &text).is_empty(),
            "{text}"
        );
    }
}

#[test]
fn dictionary_in_a_generic_container_reports_at_its_use() {
    for (container, inline_col, named_col) in [
        ("string | T", 24, 17),
        ("[T]", 16, 17),
        ("T[]", 15, 17),
        ("Readonly<T>", 15, 15),
        ("Record<string, T>", 30, 17),
        ("T & {}", 15, 17),
    ] {
        for (value, reports) in [
            ("Record<string, unknown>", true),
            ("Record<string, number>", false),
        ] {
            let direct = format!("type Result = {};", container.replace('T', value));
            let named = format!("type W<T> = {container};\ntype Result = W<{value}>;");
            for (source, position) in [
                (direct, (1, inline_col, None)),
                (named, (2, named_col, None)),
            ] {
                let hits =
                    single::run_rule("ts/no-unsafe-dictionary-type", "container.ts", &source);
                assert_eq!(
                    positions(&hits),
                    reports.then_some(position).into_iter().collect::<Vec<_>>(),
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn nested_dictionary_reports_at_the_node_that_makes_it() {
    let declarations = "type Shared<T> = Record<string, T>;\ntype Wrap<T> = { a: Record<string, T> };\ntype Slot<T> = T;";
    for (named, named_col, twin, twin_col) in [
        (
            "Shared<Shared<unknown>>",
            21,
            "Record<string, Record<string, unknown>>",
            29,
        ),
        (
            "Wrap<Shared<unknown>>",
            19,
            "{ a: Record<string, Record<string, unknown>> }",
            34,
        ),
        ("Wrap<unknown>", 14, "{ a: Record<string, unknown> }", 19),
        ("Slot<Shared<unknown>>", 14, "Record<string, unknown>", 14),
    ] {
        for (body, col) in [(named, named_col), (twin, twin_col)] {
            let source = format!("{declarations}\ntype Plain = {body};");
            let hits = single::run_rule("ts/no-unsafe-dictionary-type", "types.ts", &source);
            assert_eq!(positions(&hits), [(4, col, None)], "{body}");
        }
    }
}

#[test]
fn a_discarded_dictionary_argument_still_reports_where_it_is_written() {
    for (body, expected) in [
        ("First<number, Shared<unknown>>", vec![(3, 29, None)]),
        (
            "First<number, Record<string, unknown>>",
            vec![(3, 29, None)],
        ),
        ("number", vec![]),
    ] {
        let source = format!(
            "type Shared<T> = Record<string, T>;\ntype First<T, U> = T;\ntype Result = {body};"
        );
        let hits = single::run_rule("ts/no-unsafe-dictionary-type", "argument.ts", &source);
        assert_eq!(positions(&hits), expected, "{body}");
    }
}

#[test]
fn a_binding_only_reports_dictionaries_missing_from_the_written_declaration() {
    let source =
        "type Box<T> = { fixed: Record<string, unknown>; value: T };\ntype Result = Box<number>;";
    let hits = single::run_rule("ts/no-unsafe-dictionary-type", "written.ts", source);
    assert_eq!(
        hits.iter().map(|h| (h.line, h.col)).collect::<Vec<_>>(),
        [(1, 24)]
    );
}

#[test]
fn generic_index_interfaces_bind_the_written_value() {
    for (ty, expected) in [("unknown", vec![13]), ("number", vec![])] {
        for body in [format!("Index<{ty}>"), format!("{{[key: string]: {ty}}}")] {
            let source = format!("interface Index<T> {{ [key: string]: T }}\ntype Dict = {body};");
            let hits = single::run_rule("ts/no-unsafe-dictionary-type", "index.ts", &source);
            assert_eq!(
                hits.iter().map(|h| h.col).collect::<Vec<_>>(),
                expected,
                "{source}"
            );
        }
    }
}

#[test]
fn qualified_names_only_resolve_through_local_exports() {
    for (prefix, name, reports) in [
        (
            "namespace A { export type Other = string } namespace A { export type T = unknown }",
            "A.T",
            true,
        ),
        ("namespace A { type T = unknown }", "A.T", false),
        (
            "import * as External from './external'",
            "External.T",
            false,
        ),
        (
            "declare module 'external' { export type T = unknown } import * as External from 'external'",
            "External.T",
            false,
        ),
    ] {
        let source = format!("{prefix}\nfunction f(v: {name}) {{}}");
        let hits = single::run_rule("ts/no-unknown-parameters", "qualified.ts", &source);
        assert_eq!(
            positions(&hits),
            reports
                .then_some((2, 15, None))
                .into_iter()
                .collect::<Vec<_>>(),
            "{source}"
        );
    }
}

#[test]
fn import_equals_parameters_follow_the_exported_type() {
    for (declaration, annotation) in [
        (
            "namespace N { export type X = unknown; }\nimport Y = N.X;",
            "Y",
        ),
        ("type X = unknown;\n", "X"),
        ("\n", "unknown"),
    ] {
        let source = format!("{declaration}\nfunction read(value: {annotation}) {{}}");
        let hits = single::run_rule("ts/no-unknown-parameters", "import.ts", &source);
        assert_eq!(positions(&hits), [(3, 22, None)], "{source}");
    }
}

#[test]
fn guarding_an_imported_unknown_value_preserves_its_type() {
    for annotation in ["Y", "unknown"] {
        let source = format!(
            "namespace N {{ export type X = unknown; }}\nimport Y = N.X;\ndeclare function isString(input: unknown): input is string;\nfunction read(value: {annotation}) {{ return isString(value); }}"
        );
        let hits = single::run_rule("ts/no-known-value-widening", "guard.ts", &source);
        assert!(hits.is_empty(), "{source}");
    }
}

#[test]
fn generic_declaration_identity_includes_bound_arguments() {
    for (local, target, expected) in [
        ("type Pass<T> = Owner<T>;", "Pass<Item>", true),
        ("type Alias = Item;", "Owner<Alias>", true),
        ("interface Owner<T> { name: T }", "Owner<Item>", false),
        ("interface Other {}", "Owner<Other>", false),
        ("", "Owner", true),
    ] {
        let source = format!(
            "interface Item {{}} interface Owner<T = Item> {{ id: T }}\ndeclare const origin: Owner<Item>;\nconst v: object = origin;\n{{ {local} const picked = v as {target}; }}"
        );
        let hits = single::run_rule("ts/no-widen-then-assert", "owner.ts", &source);
        assert_eq!(hits.len(), usize::from(expected), "{source}");
    }
}

#[test]
fn a_plain_qualified_alias_reports_the_written_dictionary_once() {
    for (declaration, name) in [
        ("type X = Record<string, unknown>;", "X"),
        (
            "namespace N { export type X = Record<string, unknown>; }",
            "N.X",
        ),
        (
            "namespace A { export namespace B { export type X = Record<string, unknown>; } }",
            "A.B.X",
        ),
        (
            "namespace N { export type X = Record<string, unknown>; } import Y = N.X;",
            "Y",
        ),
    ] {
        let source = format!("{declaration}\nexport const a: {name} = {{}};");
        let hits = single::run_rule("ts/no-unsafe-dictionary-type", "alias.ts", &source);
        assert_eq!(
            positions(&hits),
            [(1, declaration.find("Record").unwrap() + 1, None)],
            "{source}"
        );
    }
}

#[test]
fn type_lookup_ignores_value_declaration_order() {
    let mut differences = Vec::new();
    for value in [
        "const Index = 1;",
        "let Index = 1;",
        "function Index() {}",
        "class Index {}",
        "namespace Index {export const x = 1}",
    ] {
        for (owner, usage) in [
            ("interface Index<T> { [key:string]: T }", "Index<unknown>"),
            ("interface Index { [key:string]: unknown }", "Index"),
            ("type Index<T> = Record<string,T>;", "Index<unknown>"),
            ("type Index = Record<string,unknown>;", "Index"),
        ] {
            let verdict = |prefix: &str| {
                let text = format!("{prefix}\nexport const v: {usage} = {{}};");
                single::run_rule("ts/no-unsafe-dictionary-type", "order.ts", &text)
                    .into_iter()
                    .filter(|h| h.line == 2)
                    .map(|h| (h.col, h.note))
                    .collect::<Vec<_>>()
            };
            let before = verdict(&format!("{value} {owner}"));
            let after = verdict(&format!("{owner} {value}"));
            if before != after {
                differences.push(format!("{value} {owner}: {before:?}, {after:?}"));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn identity_follows_named_types_in_composite_positions() {
    let mut differences = Vec::new();
    for shape in [
        "T[]",
        "readonly T[]",
        "[T]",
        "T | string",
        "T & {tag:1}",
        "(T)",
        "{p:T}",
        "(v:T)=>string",
        "()=>T",
        "Container<T>",
    ] {
        let literal = shape.replace('T', "Model");
        let named = shape.replace('T', "Id<Model>");
        for (left, right) in [(&literal, &named), (&named, &literal), (&named, &named)] {
            let source = format!(
                "interface Model {{id:number}} interface Container<T> {{item:T}} type Id<T>=T;\ndeclare const original: {left};\nconst v: Record<string,unknown> = original;\nconst picked = v as {right};"
            );
            let hits = single::run_rule("ts/no-widen-then-assert", "equal.ts", &source);
            if !hits
                .iter()
                .any(|h| (h.line, h.col, h.note.as_deref()) == (4, 16, None))
            {
                differences.push(format!("{left} to {right}"));
            }
        }
    }
    for left in ["Id<Model>[]", "Model[]"] {
        for right in ["Pass<Model>[]", "Model[]"] {
            let source = format!(
                "interface Model {{ id: number }}\ntype Id<T> = T;\ntype Pass<T> = T;\ndeclare const original: {left};\nconst wide: Record<string,unknown> = original;\nconst back = wide as {right};"
            );
            let hits = single::run_rule("ts/no-widen-then-assert", "two-names.ts", &source);
            assert_eq!(positions(&hits), [(6, 14, None)], "{left} to {right}");
        }
    }
    for (left, right, same) in [
        ("Arr<Model>", "Arr<Other>", false),
        ("Arr<Model>", "Arr<Model>", true),
        ("Arr<Model>", "Model[]", true),
        ("Model | Other", "Other | Model", false),
        ("Model & Other", "Other & Model", false),
    ] {
        let source = format!(
            "interface Model {{id:number}} interface Other {{extra:string}} type Arr<T>=T[];\ndeclare const original: {left};\nconst v: Record<string,unknown> = original;\nconst picked = v as {right};"
        );
        let hits = single::run_rule("ts/no-widen-then-assert", "equal.ts", &source);
        if hits.is_empty() == same {
            differences.push(format!("{left} to {right}"));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn readonly_identity_preserves_the_spelled_out_verdict() {
    for (source_type, target_type, reports) in [
        ("T[]", "readonly T[]", true),
        ("readonly T[]", "T[]", true),
        ("readonly [T]", "[T]", true),
        ("[T]", "readonly [T]", true),
        ("ReadonlyArray<T>", "T[]", false),
        ("T[]", "ReadonlyArray<T>", false),
        ("readonly T[][]", "T[][]", true),
    ] {
        for left in ["Model", "Id<Model>"] {
            for right in ["Model", "Id<Model>"] {
                let left = source_type.replace('T', left);
                let right = target_type.replace('T', right);
                let source = format!(
                    "interface Model {{ id: number }}\ntype Id<T> = T;\ndeclare const original: {left};\nconst wide: Record<string, unknown> = original;\nconst back = wide as {right};"
                );
                let hits = single::run_rule("ts/no-widen-then-assert", "readonly.ts", &source);
                assert_eq!(
                    positions(&hits),
                    if reports { vec![(5, 14, None)] } else { vec![] },
                    "{left} to {right}"
                );
            }
        }
    }
}

#[test]
fn array_identity_distinguishes_shadowed_declarations() {
    for inner in ["Model", "Other"] {
        for shape in ["", "[]"] {
            for source in ["Model", "Id<Model>"] {
                for target in [inner.to_owned(), format!("Id<{inner}>")] {
                    let source = format!(
                        "interface Model {{ id: number }}\ntype Id<T> = T;\ndeclare const original: {source}{shape};\nconst wide: Record<string, unknown> = original;\n{{\n  interface {inner} {{ label: string }}\n  const back = wide as {target}{shape};\n}}"
                    );
                    let hits = single::run_rule("ts/no-widen-then-assert", "scope.ts", &source);
                    assert!(hits.is_empty(), "{source}");
                }
            }
        }
    }
}

#[test]
fn identity_aliases_of_a_function_parameter_report_at_the_assertion() {
    for left in ["Other", "Id<Other>"] {
        for right in ["Other", "Id<Other>"] {
            let source = format!(
                "interface Model {{ id: number }}\ntype Id<T> = T;\nfunction f<Other>(original: {left}[]) {{\n const wide: Record<string, unknown> = original;\n const back = wide as {right}[];\n}}"
            );
            let hits = single::run_rule("ts/no-widen-then-assert", "parameter.ts", &source);
            assert_eq!(positions(&hits), [(5, 15, None)], "{source}");
        }
    }
}

fn assert_site(rule: &str, source: &str, site: Option<(usize, usize)>) {
    let hits = single::run_rule(rule, "types.ts", source);
    let expected = site.map(|(line, col)| (line, col, None));
    assert_eq!(positions(&hits), expected.as_slice(), "{source}");
}

#[test]
fn inline_signatures_pair_their_own_type_parameters() {
    for (origin, target, reports) in [
        ("Call", "Call", true),
        ("<T>(value: T) => T", "<T>(value: T) => T", true),
        ("<T>(value: T) => T", "<U>(value: U) => U", true),
        ("{ <T>(value: T): T }", "{ <U>(value: U): U }", true),
        ("<T, U>(value: T) => U", "<U, T>(value: T) => U", false),
        (
            "<T extends string>(value: T) => T",
            "<U extends number>(value: U) => U",
            false,
        ),
        (
            "<T>(value: T) => <U>() => T",
            "<U>(value: U) => <V>() => U",
            true,
        ),
        (
            "<T>(value: T) => <U>() => T",
            "<T>(value: T) => <T>() => T",
            false,
        ),
        ("<T>() => Model", "<U>() => Other", false),
    ] {
        let source = format!(
            "type Call = <T>(value: T) => T; interface Model {{}} interface Other {{}}\ndeclare const origin: {origin};\nconst wide: Record<string, unknown> = origin;\nconst back = wide as {target};"
        );
        assert_site(
            "ts/no-widen-then-assert",
            &source,
            reports.then_some((4, 14)),
        );
    }
}

#[test]
fn tuple_elements_keep_the_bound_dictionary_value() {
    for element in ["item: V", "V?", "item?: V", "...V[]", "...items: V[]"] {
        for (value, reports) in [("number", false), ("unknown", true)] {
            let tuple = format!("[{}]", element.replace('V', "Record<string, T>"));
            for (source, col) in [
                (
                    format!(
                        "\ntype Result = {};",
                        tuple.replace(", T>", &format!(", {value}>"))
                    ),
                    15 + tuple.find("Record").unwrap(),
                ),
                (
                    format!("type Wrap<T> = {tuple};\ntype Result = Wrap<{value}>;"),
                    15,
                ),
            ] {
                assert_site(
                    "ts/no-unsafe-dictionary-type",
                    &source,
                    reports.then_some((2, col)),
                );
            }
        }
    }
}

#[test]
fn nested_merged_namespace_exports_are_independent_of_order() {
    for (export, ty, reports) in [
        ("export ", "number", false),
        ("", "unknown", false),
        ("export ", "unknown", true),
    ] {
        let value = format!(
            "namespace Outer {{ export namespace Inner {{ {export}type Value = {ty}; }} }}"
        );
        let other = "namespace Outer { export namespace Inner { export type Other = string; export const Value = 1; } }";
        for (a, b) in [(value.as_str(), other), (other, value.as_str())] {
            for (import, name) in [
                ("", "Outer.Inner.Value"),
                ("import I = Outer.Inner; ", "I.Value"),
            ] {
                let source = format!("{a}\n{b}\n{import}function read(x: {name}) {{}}");
                assert_site(
                    "ts/no-unknown-parameters",
                    &source,
                    reports.then_some((3, import.len() + 18)),
                );
            }
        }
    }
}

#[test]
fn inherited_index_values_keep_the_heritage_arguments() {
    for (value, reports) in [("number", false), ("never", true)] {
        for child in [
            "interface Child<T> { [key: string]: { only?: T } }",
            "interface Child<T> extends Base<T> {}",
            "interface Middle<U> extends Base<U> {} interface Child<T> extends Middle<T> {}",
        ] {
            let source = format!(
                "interface Base<T> {{ [key: string]: {{ only?: T }} }}\n{child}\ntype Dict = Child<{value}>;"
            );
            assert_site(
                "ts/no-unsafe-dictionary-type",
                &source,
                reports.then_some((3, 13)),
            );
        }
    }
}
