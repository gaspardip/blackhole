use blackhole_check::engine::{registry, run};

const WRITES: &[(&str, bool)] = &[
    ("let X=T; X=C; U", false),
    ("var X=T; var X=C; U", false),
    ("let X; X=T; X=C; U", false),
    ("let X=C; X=T; U", true),
    ("let X=T; if(flag) X=C; U", true),
    ("let X=T; flag ? X=C : nothing; U", true),
    ("let X=T; X &&= C; U", true),
    ("let X=T; X ||= C; U", true),
    ("let X=T; X ??= C; U", true),
    ("let X=T; {X=C;} U", true),
    ("let X=T; try {X=C;} catch {} U", true),
    ("let X=T; while(flag){X=C;} U", true),
    ("let X=T; label:{X=C;} U", true),
    ("let X; for(;;){U X=T; X=C;}", true),
    ("let X=T; X=(U C);", true),
    ("let X=T; X=C; function later(){U}", true),
    ("let X=T; X=C; const later=()=>{U};", true),
    ("let X=T; X=C; class Later {method(){U}}", true),
    ("function earlier(){U} let X=T; X=C; earlier();", true),
    ("let X=T; function again(){X=T;} X=C; again(); U", true),
    ("let X; switch(n){case 0:X=T;case 1:X=C; U}", true),
    ("let X=T; ({field:X}=unknown); U", true),
];

const OWNERS: &[(&str, &str, &str, &str)] = &[
    ("ts/no-test-doubles", "vi", "X.mock('client')", "X.mock"),
    ("ts/no-skipped-tests", "test.skip", "X('x',fn)", "X("),
    ("ts/no-fixed-sleep", "Bun.sleep", "await X(10)", "await X"),
    (
        "ts/no-test-doubles",
        "'clipboard'",
        "Reflect.set(navigator, X, config)",
        "Reflect.set",
    ),
    ("ts/strict-msw", "setupServer()", "X.listen()", "X.listen"),
];

#[test]
fn assignment_spellings() {
    for &(rule, tracked, call, anchor) in OWNERS {
        for (prefix, suffix) in [
            ("", ""),
            ("function outer(){", "}"),
            ("while(flag){", "}"),
            ("switch(n){case 1:", "}"),
        ] {
            for &(writes, reports) in WRITES {
                let timer = rule == "ts/no-fixed-sleep";
                let nested = writes.contains("later()")
                    || writes.contains("later=")
                    || writes.contains("method()")
                    || writes.contains("earlier()");
                let call = if timer && nested {
                    format!("beforeEach(async()=>{{{call}}})")
                } else {
                    call.into()
                };
                let call = call + if writes.contains("(U") { "," } else { ";" };
                let body = writes
                    .replace('T', tracked)
                    .replace(
                        'C',
                        if tracked == "'clipboard'" {
                            "'__CONFIG__'"
                        } else {
                            "custom"
                        },
                    )
                    .replace('U', &call);
                let body = if timer {
                    format!("beforeEach(async()=>{{{body}}});")
                } else {
                    body
                };
                let source =
                    format!("import {{setupServer}} from 'msw/node';{prefix}{body}{suffix}");
                let found = run(registry(), [("case.test.ts", source.as_bytes())]);
                assert!(
                    !found.iter().any(|h| h.rule == "core/unparseable"),
                    "{source}"
                );
                let columns: Vec<_> = found
                    .iter()
                    .filter(|h| h.rule == rule)
                    .map(|h| h.col)
                    .collect();
                let expected = reports.then(|| source.find(anchor).unwrap() + 1);
                assert_eq!(
                    columns,
                    expected.into_iter().collect::<Vec<_>>(),
                    "{rule}: {source}"
                );
            }
        }
    }
}
