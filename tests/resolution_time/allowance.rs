use super::Resolution;
use crate::{
    engine::{Findings, Rule, Run, Settings, TsCtx, run_with_work},
    packs::ts::{no_unsafe_dictionary_type, no_widen_then_assert},
};
use std::fmt::Write;

fn dictionary(ctx: &TsCtx<'_>, findings: &mut Findings) {
    no_unsafe_dictionary_type::check_with_resolution(
        ctx,
        findings,
        &Resolution::with_budget(256, 128),
    );
}

fn assertion(ctx: &TsCtx<'_>, findings: &mut Findings) {
    no_widen_then_assert::check_with_resolution(ctx, findings, &Resolution::with_budget(256, 128));
}

#[test]
fn twenty_thousand_annotations_and_assertions_receive_their_own_allowance() {
    let mut source = String::from("type A0<T> = T;\n");
    for n in 1..=8 {
        writeln!(
            source,
            "type A{n}<T> = A{prev}<T[]> | A{prev}<readonly T[]>;",
            prev = n - 1
        )
        .unwrap();
    }
    source.push_str("const stress: Record<A8<string>, unknown> = {id: 1};\nconst first = stress as {id: number};\n");
    let prefix = source.lines().count();
    for i in 0..20_000 {
        writeln!(source, "const v{i}: Record<string, unknown> = {{id: 1}};").unwrap();
        writeln!(source, "const x{i} = v{i} as {{id: number}};").unwrap();
    }
    let (hits, work) = run_with_work(
        &[
            Rule {
                run: Run::Ts(dictionary),
                ..no_unsafe_dictionary_type::RULE
            },
            Rule {
                run: Run::Ts(assertion),
                ..no_widen_then_assert::RULE
            },
        ],
        Settings::default(),
        &[],
        [("allowance.ts", source.as_bytes())],
    );
    let cap = source.len() as u64;
    assert!(work <= cap, "{work} work exceeds {cap}");
    for rule in [
        &no_unsafe_dictionary_type::RULE,
        &no_widen_then_assert::RULE,
    ] {
        assert!(hits.iter().any(|h| h.rule == rule.id
            && h.line <= prefix
            && h.note.as_deref() == Some("type too complex to judge")));
    }
    let ordinary: Vec<_> = hits.iter().filter(|h| h.line > prefix).collect();
    assert_eq!(ordinary.len(), 40_000);
    for (i, pair) in ordinary.as_chunks::<2>().0.iter().enumerate() {
        for (offset, h, rule) in [
            (1, pair[0], &no_unsafe_dictionary_type::RULE),
            (2, pair[1], &no_widen_then_assert::RULE),
        ] {
            assert_eq!(
                (h.line, h.rule, h.note.as_deref()),
                (prefix + 2 * i + offset, rule.id, None)
            );
        }
    }
}
