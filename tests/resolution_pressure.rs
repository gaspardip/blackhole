#[path = "common/run_rule.rs"]
mod single;
use std::fmt::Write;

const CASES: &[(&str, &str, &str, &str)] = &[
    (
        "no-unknown-parameters",
        "unknown",
        "function f(v: @P) {}",
        "function stress(v: S) {}",
    ),
    (
        "no-object-parameters",
        "object",
        "function f(v: @P) {}",
        "function stress(v: S) {}",
    ),
    (
        "no-unknown-returns",
        "unknown",
        "declare function f(): @P;",
        "declare function stress(): Promise<S>;",
    ),
    (
        "no-unknown-type-aliases",
        "unknown",
        "type @Victim = P;",
        "type Load = S;",
    ),
    (
        "no-unsafe-dictionary-type",
        "unknown",
        "const x: @Record<string, P> = {};",
        "type Load = Record<string, S>;",
    ),
    (
        "no-known-value-widening",
        "unknown",
        "const v: P = @1;",
        "const s: Record<S, unknown> = {id:1};",
    ),
    (
        "no-known-value-widening",
        "unknown",
        "declare function f(v: P): v is string; f(@'x');",
        "declare function stress(v: S): v is string; stress('x');",
    ),
    (
        "no-widen-then-assert",
        "Record<string, unknown>",
        "const v: P = {id:1}; const x = @v as {id:number};",
        "const s: Record<S,unknown> = {id:1}; const x = s as {id:number};",
    ),
    (
        "no-widen-then-assert",
        "unknown",
        "declare const origin: P; const v: unknown = origin; const x = @v as {id:number};",
        "const s: Record<S,unknown> = {id:1}; const x = s as {id:number};",
    ),
    (
        "no-widen-then-assert",
        "{id:number}",
        "const v: object = {id:1}; const x = @v as P;",
        "const s: Record<S,unknown> = {id:1}; const x = s as {id:number};",
    ),
];

#[test]
fn each_semantic_question_preserves_its_answer_or_reports_exhaustion() {
    let mut chain = String::from("type B0<T> = T;\n");
    for i in 1..=24 {
        writeln!(
            chain,
            "type B{i}<T> = B{prev}<T[]> | B{prev}<readonly T[]>;",
            prev = i - 1
        )
        .unwrap();
    }
    let mut differences = Vec::new();
    for (rule, endpoint, victim, stress) in CASES {
        let id = format!("ts/{rule}");
        let marker = victim.find('@').unwrap() + 3;
        let victim = format!("{{ {} }}", victim.replace('@', ""));
        let prefix = format!("type End = {endpoint}; type P = End;\n");
        let control = single::run_rule(&id, "control.ts", &format!("{prefix}{victim}"));
        let expected = control
            .iter()
            .any(|h| h.line == 2 && h.col == marker && h.note.is_none());
        for (column, ty, between) in [
            ("before", "B24<string>", false),
            ("between", "B24<string>", true),
            ("shared union", "B24<string> | P", false),
            ("shared argument", "B24<P>", false),
        ] {
            let mut source = format!("{prefix}{chain}");
            if between {
                writeln!(source, "{victim}").unwrap();
            }
            let stress = stress
                .replace(" S", &format!(" {ty}"))
                .replace("<S", &format!("<{ty}"));
            writeln!(source, "{{ {stress} }}").unwrap();
            let line = source.lines().count() + 1;
            source.push_str(&victim);
            let hits = single::run_rule(&id, "pressure.ts", &source);
            let got: Vec<_> = hits
                .iter()
                .filter(|h| h.line == line && h.col == marker)
                .map(|h| h.note.as_deref())
                .collect();
            if got != [Some("type too complex to judge")]
                && got != expected.then_some(None).into_iter().collect::<Vec<_>>()
            {
                differences.push(format!(
                    "{rule} {victim} {column}: {got:?}, pressure-free {expected}"
                ));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
