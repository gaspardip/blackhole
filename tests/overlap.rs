#[path = "common/run_rule.rs"]
mod rule_runner;
use rule_runner::run_rule;

struct Case {
    silent: &'static str,
    path: &'static str,
    snippet: &'static str,
    reporting: &'static str,
}

const CASES: &[Case] = &[
    Case {
        silent: "ts/no-known-value-widening",
        path: "valid-06.ts",
        snippet: "type Command = () => void; const startCommand = () => {}; function create(): Record<string, Command> { return {}; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-known-value-widening",
        path: "valid-07.ts",
        snippet: "type Command = () => void; const startCommand = () => {}; const create = (): Record<string, Command> => ({});",
        reporting: "ts/no-explicit-return-types",
    },
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
    Case {
        silent: "ts/no-reflect-apply",
        path: "valid-02.ts",
        snippet: "Reflect.get(owner, key);",
        reporting: "ts/no-reflect-get",
    },
    Case {
        silent: "ts/no-comments",
        path: "preserves-exported-public-api-documentation-12.ts",
        snippet: "interface Result { /** Number of items. */ count: number; }\nexport function count(): Result { return { count: 1 }; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-explicit-return-types",
        path: "allows-inferred-returns-and-type-only-signatures-01.ts",
        snippet: "function isString(value: unknown): value is string { return typeof value === 'string'; }",
        reporting: "ts/no-runtime-typeof",
    },
    Case {
        silent: "ts/no-explicit-return-types",
        path: "allows-inferred-returns-and-type-only-signatures-02.ts",
        snippet: "function assertString(value: unknown): asserts value is string { if (typeof value !== 'string') throw Error(); }",
        reporting: "ts/no-runtime-typeof",
    },
    Case {
        silent: "ts/no-known-value-widening",
        path: "valid-18.ts",
        snippet: "type Command = () => void; const startCommand = () => {}; interface Commands { readonly start: Command } function create(): Commands { return { start: startCommand }; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-parameters",
        path: "valid-01.ts",
        snippet: "function enrich(cause: unknown): void {}",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-parameters",
        path: "valid-02.ts",
        snippet: "function enrich(cause: Error | unknown): void {}",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-parameters",
        path: "valid-09.ts",
        snippet: "function parse(value: string | number): void {}",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-02.ts",
        snippet: "function parse(): ImportedValue { return input; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-03.ts",
        snippet: "function parse(): User { return user; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-05.ts",
        snippet: "function generic<Value>(): Value { return value; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-10.ts",
        snippet: "type Result = { value: unknown }; function load(): Result { return result; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-11.ts",
        snippet: "function load(): Promise<User> { return promise; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unknown-returns",
        path: "valid-12.ts",
        snippet: "type Identity<T> = T; function load(): Identity<User> { return user; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-unsafe-dictionary-type",
        path: "valid-25.ts",
        snippet: "function run<T extends Record<string, unknown>>(input: T): T { return input; }",
        reporting: "ts/no-explicit-return-types",
    },
    Case {
        silent: "ts/no-known-value-widening",
        path: "overload.ts",
        snippet: "function guard(value: unknown): value is string;\nfunction guard(value: unknown, mode: 'boolean'): boolean;\nfunction guard(value: unknown, mode?: 'boolean') { return true; }\nguard('x', 'boolean');",
        reporting: "ts/no-unknown-parameters",
    },
];

#[test]
fn upstream_accept_cases_with_other_findings() {
    let mut failures = Vec::new();
    for case in CASES {
        for (id, expected_clean) in [(case.silent, true), (case.reporting, false)] {
            let hits = run_rule(id, case.path, case.snippet);
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
