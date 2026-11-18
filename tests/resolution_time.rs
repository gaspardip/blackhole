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
