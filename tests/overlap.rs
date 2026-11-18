use blackhole_check::engine::{registry, run};

struct Case {
    silent: &'static str,
    path: &'static str,
    snippet: &'static str,
    reporting: &'static str,
}

const CASES: &[Case] = &[
    Case {
        silent: "ts/no-widen-then-assert",
        path: "valid-01.ts",
        snippet: "const source = { id: 'first' }; const widened: unknown = source;",
        reporting: "ts/no-known-value-widening",
    },
    Case {
        silent: "ts/no-known-value-widening",
        path: "valid-28.ts",
        snippet: "function isString(value: unknown): value is string { return true; } declare function readInput(): unknown; isString(readInput());",
        reporting: "ts/no-unknown-returns",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-09.ts",
        snippet: "function cause(): { cause: unknown } { return { cause: input }; }",
        reporting: "ts/no-known-value-widening",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-01.ts",
        snippet: "type ImportedValue = unknown;",
        reporting: "ts/no-unknown-type-aliases",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-06.ts",
        snippet: "type Value = unknown; function generic<Value>(): Value { return value; }",
        reporting: "ts/no-unknown-type-aliases",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-07.ts",
        snippet: "type Key = unknown; type Mapped<Input> = { [Key in keyof Input]: () => Key };",
        reporting: "ts/no-unknown-type-aliases",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-08.ts",
        snippet: "type Item = unknown; type Unpacked<Input> = Input extends Promise<infer Item> ? () => Item : never;",
        reporting: "ts/no-unknown-type-aliases",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-13.ts",
        snippet: "type Value = unknown; function outer() { type Value = User; function load(): Value { return user; } }",
        reporting: "ts/no-unknown-type-aliases",
    },
];

#[test]
fn upstream_accept_cases_with_other_findings() {
    let mut failures = Vec::new();
    for case in CASES {
        for (id, expected_clean) in [(case.silent, true), (case.reporting, false)] {
            let rule = registry()
                .iter()
                .find(|r| r.id == id)
                .unwrap_or_else(|| panic!("missing rule {id}"));
            let hits = run(
                std::slice::from_ref(rule),
                [(case.path, case.snippet.as_bytes())],
            );
            if hits.is_empty() != expected_clean {
                failures.push(format!(
                    "{id} reported {} findings for {}: {}",
                    hits.len(),
                    case.path,
                    case.snippet
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
