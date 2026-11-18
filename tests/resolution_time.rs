use blackhole_check::engine::{registry, run};
use std::{
    fmt::Write,
    time::{Duration, Instant},
};

#[test]
fn repeated_applications_share_their_default_bindings() {
    for (parameters, uses) in [(2000, 48), (4000, 100)] {
        let mut source = String::from("type A<T0 = unknown");
        for i in 1..parameters {
            write!(source, ", T{i} = T0").unwrap();
        }
        source.push_str("> = T0;\n");
        for i in 0..uses {
            writeln!(source, "function f{i}(x: A) {{}}").unwrap();
        }
        let start = Instant::now();
        let hits = run(registry(), [("defaults.ts", source.as_bytes())]);
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(hits.len(), uses);
        for (index, h) in hits.iter().enumerate() {
            assert_eq!(
                (h.rule, h.line, h.note.as_deref()),
                ("ts/no-unknown-parameters", index + 2, None)
            );
        }
    }
}

#[test]
fn large_declarations_and_repeated_local_lookups_finish_within_budget() {
    for (declarations, uses) in [(8000, 2), (2, 8000)] {
        let mut literal = String::from("type Big<T> = {\n");
        let mut namespace = String::new();
        for index in 0..declarations {
            writeln!(literal, "m{index}: number;").unwrap();
            writeln!(
                namespace,
                "namespace Local {{ export type X{index} = number; }}"
            )
            .unwrap();
        }
        literal.push_str("last: T };\n");
        let last = declarations - 1;
        for index in 0..uses {
            writeln!(literal, "declare const l{index}: Big<number>;").unwrap();
            writeln!(namespace, "declare const n{index}: Local.X{last};").unwrap();
        }
        for (path, source) in [("literal.ts", literal), ("namespace.ts", namespace)] {
            let start = Instant::now();
            let hits = run(registry(), [(path, source.as_bytes())]);
            assert!(hits.is_empty(), "{path}: {} findings", hits.len());
            assert!(start.elapsed() < Duration::from_secs(3), "{path}");
        }
    }
}
