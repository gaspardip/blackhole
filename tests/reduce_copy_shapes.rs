use blackhole_check::engine::{registry, run};

const RULE_ID: &str = "ts/no-reduce-accumulator-copy";

fn check(path: &str, source: &str, expected: usize) {
    let hits = run(registry(), [(path, source.as_bytes())]);
    assert_eq!(
        hits.iter()
            .filter(|hit| matches!(hit.rule, RULE_ID | "core/unparseable"))
            .map(|hit| hit.rule)
            .collect::<Vec<_>>(),
        vec![RULE_ID; expected],
        "{source}"
    );
}

fn reports(source: &str) {
    check("shape.ts", source, 1);
}

#[test]
fn listed_copy_shapes_report_once() {
    let mut copies = vec![
        ("", "({...acc})".to_owned(), "", "{}"),
        ("", "[...acc]".to_owned(), "", "[]"),
        (
            "",
            "Object.assign({}, base, acc, item, ...layers)".to_owned(),
            "",
            "{}",
        ),
        ("", "Object.assign([], acc)".to_owned(), "", "[]"),
        (
            "const box = { next: {} };",
            "Object.assign(box.next, acc)".to_owned(),
            "",
            "{}",
        ),
        (
            "const box = { next: [] }; const key = 'next';",
            "Object.assign(box[key], acc)".to_owned(),
            "",
            "[]",
        ),
    ];
    for kind in ["let", "const", "var"] {
        for seed in ["{}", "[]"] {
            reports(&format!(
                "rows.reduce(acc => {{ {kind} next = {seed}; return Object.assign(next, acc); }}, {seed});"
            ));
        }
    }
    for copy in [
        "Array.from(acc)",
        "acc.concat(item)",
        "acc.slice()",
        "acc.toSpliced(0, 1)",
        "acc.toSorted()",
        "acc.toReversed()",
        "acc.with(0, item)",
    ] {
        copies.push(("", copy.to_owned(), "", "[]"));
        copies.push(("", copy.to_owned(), ": number[]", "unknown"));
    }

    for method in ["reduce", "reduceRight"] {
        for (setup, copy, annotation, seed) in &copies {
            let params = format!("(acc{annotation}, item)");
            let body = format!("{{ {setup} return {copy}; }}");
            for (declaration, callback) in [
                (String::new(), format!("{params} => {body}")),
                (format!("function fold{params} {body}"), "fold".to_owned()),
                (
                    format!(
                        "function fold(acc: object, item: number): object; function fold{params} {body}"
                    ),
                    "fold".to_owned(),
                ),
                (
                    format!("const fold = {params} => {body}; const alias = fold;"),
                    "alias".to_owned(),
                ),
            ] {
                reports(&format!("{declaration} rows.{method}({callback}, {seed});"));
            }
            for wrapper in [
                "COPY",
                "(() => COPY)()",
                "(() => COPY).call(null)",
                "Reflect.apply(() => COPY, null, [])",
                "(() => COPY).bind(null)()",
            ] {
                let copy = wrapper.replace("COPY", copy);
                for body in [
                    format!("const next = {copy}; acc = replacement; return next;"),
                    format!("acc = {copy}; return acc;"),
                ] {
                    reports(&format!(
                        "rows.{method}({params} => {{ {setup} {body} }}, {seed});"
                    ));
                }
            }
        }

        for call in [
            format!("rows.{method}"),
            format!("rows?.{method}"),
            format!("rows[\"{method}\"]"),
            format!("rows[`{method}`]"),
            format!("(rows?.{method}!)"),
        ] {
            for optional in ["", "?."] {
                reports(&format!("{call}{optional}(acc => [...acc], []);"));
            }
        }
        for receiver in [
            "(rows as number[])",
            "(rows!)",
            "(rows satisfies number[])",
            "(<number[]>rows)",
        ] {
            reports(&format!("{receiver}.{method}(acc => [...acc], []);"));
        }
    }
}

#[test]
fn incompatible_writes_and_wrappers_keep_copies_silent() {
    for before in [
        "[acc] = [item];",
        "({ acc } = item);",
        "acc++;",
        "acc += item;",
        "for (acc of items) {}",
        "for (acc in item) {}",
        "(acc as number[]) = item;",
        "acc! = item;",
        "(() => { acc = item; })();",
        "while (enabled) { acc = item; }",
        "acc = item;",
        "const prev = acc; acc = item;",
    ] {
        reports("rows.reduce((acc, item) => { return [...acc]; }, []);");
        let value = if before.starts_with("const prev") {
            "prev"
        } else {
            "acc"
        };
        check(
            "shape.ts",
            &format!("rows.reduce((acc, item) => {{ {before} return [...{value}]; }}, []);"),
            0,
        );
    }

    for (accepted, broken) in [
        ("(() => [...acc])()", "defer(() => [...acc])"),
        (
            "Reflect.apply(() => [...acc], null, [])",
            "(() => [...acc]).apply(null, [])",
        ),
        (
            "(function () { return [...acc]; })()",
            "(function* () { return [...acc]; })()",
        ),
        (
            "(function () { return [...acc]; })()",
            "new (function () { return [...acc]; })()",
        ),
        ("(() => [...acc])()", "(other ?? (() => [...acc]))()"),
        ("(() => [...acc])()", "(other || (() => [...acc]))()"),
        ("(() => [...acc])()", "(other && (() => [...acc]))()"),
        (
            "Object.assign({}, acc)",
            "Object.assign({}, ...layers, acc)",
        ),
        (
            "Object.assign({}, acc)",
            "Object.assign({}, ...[base], acc)",
        ),
        ("Object.assign({}, acc)", "Object.assign({}, ...[], acc)"),
    ] {
        reports(&format!("rows.reduce(acc => {accepted}, []);"));
        check("shape.ts", &format!("rows.reduce(acc => {broken}, []);"), 0);
    }
}

#[test]
fn non_fresh_assign_targets_keep_copies_silent() {
    for (accepted, broken) in [
        (
            "const next = {}; return Object.assign(next, acc);",
            "return Object.assign(shared, acc);",
        ),
        (
            "var next = {}; return Object.assign(next, acc);",
            "if (flag) { var next = {}; } return Object.assign(next, acc);",
        ),
        (
            "let next = {}; return Object.assign(next, acc);",
            "let next = {}; const result = Object.assign(next, acc); next = shared; return result;",
        ),
        (
            "const next = {}; return Object.assign(next, acc);",
            "const next = {}; const alias = next; return Object.assign(alias, acc);",
        ),
        (
            "let next = {}; return Object.assign(next, acc);",
            "let next = {}; defer(() => { next = shared; }); return Object.assign(next, acc);",
        ),
        (
            "const box = { next: {} }; return Object.assign(box.next, acc);",
            "const box = { next: {} }; defer(() => { box.next = shared; }); return Object.assign(box.next, acc);",
        ),
        (
            "let next = {}; return Object.assign(next, acc);",
            "let next = {}; eval('next = shared'); return Object.assign(next, acc);",
        ),
        (
            "const next = {}; return Object.assign(next, acc);",
            "type next = object; const next = {}; return Object.assign(next, acc);",
        ),
    ] {
        reports(&format!("rows.reduce(acc => {{ {accepted} }}, {{}});"));
        check(
            "shape.ts",
            &format!("rows.reduce(acc => {{ {broken} }}, {{}});"),
            0,
        );
    }
}

#[test]
fn suspension_missing_evidence_and_ambiguity_keep_copies_silent() {
    for suspend in [
        "await ready;",
        "for await (const value of input) {}",
        "await using handle = open();",
        "function* pending() { yield 0; }",
    ] {
        reports("rows.reduce(async acc => { return [...acc]; }, []);");
        check(
            "shape.ts",
            &format!("rows.reduce(async acc => {{ {suspend} return [...acc]; }}, []);"),
            0,
        );
    }

    for copy in [
        "Array.from(acc)",
        "acc.concat(item)",
        "acc.slice()",
        "acc.toSpliced(0, 1)",
        "acc.toSorted()",
        "acc.toReversed()",
        "acc.with(0, item)",
    ] {
        reports(&format!("rows.reduce((acc, item) => {copy}, []);"));
        check(
            "shape.ts",
            &format!("rows.reduce((acc, item) => {copy}, unknown);"),
            0,
        );
    }

    for (path, accepted, broken) in [
        (
            "shape.cjs",
            "rows.reduce(acc => [...acc], []);",
            "with (scope) {} rows.reduce(acc => [...acc], []);",
        ),
        (
            "shape.ts",
            "rows.reduce(acc => [...acc], []);",
            "rows.reduce(acc => { interface acc {} return [...acc]; }, []);",
        ),
        (
            "shape.ts",
            "rows.reduce(acc => [...acc], []);",
            "rows.reduce(function* (acc) { return [...acc]; }, []);",
        ),
        (
            "shape.ts",
            "rows.reduce(acc => [...acc], []);",
            "rows.reduce((acc, x = [...acc]) => acc, []);",
        ),
        (
            "shape.ts",
            "rows.reduce(acc => { const box = { next: {} }; return Object.assign(box.next, acc); }, {});",
            "declare const eval: unknown; rows.reduce(acc => { const box = { next: {} }; return Object.assign(box.next, acc); }, {});",
        ),
        (
            "shape.ts",
            "const method = 'reduce'; rows[method](acc => [...acc], []);",
            "declare const eval: unknown; const method = 'reduce'; rows[method](acc => [...acc], []);",
        ),
        (
            "shape.cjs",
            "rows.reduce(acc => { var next = {}; return Object.assign(next, acc); }, {});",
            "rows.reduce(acc => { var next = {}; { function next() {} } return Object.assign(next, acc); }, {});",
        ),
    ] {
        check(path, accepted, 1);
        check(path, broken, 0);
    }
}

#[test]
fn work_exhaustion_stops_one_file_without_affecting_the_next() {
    let key = "a".repeat(8192);
    let bounded = format!(
        "rows.reduce(acc => {{
const early = [...acc];
const box = {{'{key}': acc}};
const copy = [...box['{key}']];
return [...acc];
}}, []);"
    );
    let after = "rows.reduce(acc => [...acc], []);";
    let hits = run(
        registry(),
        [("a.ts", bounded.as_bytes()), ("b.ts", after.as_bytes())],
    );
    let reports: Vec<_> = hits
        .iter()
        .filter(|hit| matches!(hit.rule, RULE_ID | "core/unparseable"))
        .map(|hit| (hit.path.as_str(), hit.line, hit.rule))
        .collect();
    assert_eq!(reports, [("a.ts", 2, RULE_ID), ("b.ts", 1, RULE_ID)]);
}
