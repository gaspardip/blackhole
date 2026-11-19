use blackhole_check::{
    engine::Finding,
    ledger::Counts,
    objections::Rows,
    verdict::{Approvals, Change, Inputs, Kind, judge},
};
use globset::{Glob, GlobSetBuilder};

#[test]
fn last_substantive_review() {
    for (login, state, head, approved) in [
        ("Peer", "COMMENTED", "head", true),
        ("PEER", "CHANGES_REQUESTED", "head", false),
        ("author", "APPROVED", "head", false),
        ("peer", "APPROVED", "base", false),
    ] {
        let first = if login == "author" { "AUTHOR" } else { "peer" };
        let data = serde_json::json!({"authors":["author"],"reviews":[{"login":first,"state":"APPROVED","commit":"head","permission":"write"},{"login":login,"state":state,"commit":head,"permission":"write"}]});
        let reviews: Approvals = serde_json::from_value(data).unwrap();
        assert_eq!(reviews.approved("head"), approved);
    }
}

#[test]
fn gate_table() {
    let excludes = GlobSetBuilder::new()
        .add(Glob::new("vendor/**").unwrap())
        .build()
        .unwrap();
    for (path, from, old, new, found, expected) in [
        ("x", None, 0, 0, 0, ""),
        (".github/x", None, 0, 0, 0, "protected"),
        ("app/tsconfig.strict.json", None, 0, 0, 0, "protected"),
        ("app/eslint.config.mjs", None, 0, 0, 0, "protected"),
        ("app/.oxlintrc.json", None, 0, 0, 0, "protected"),
        ("app/.prettierrc", None, 0, 0, 0, "protected"),
        ("biome.jsonc", None, 0, 0, 0, "protected"),
        ("vitest.config.mts", None, 0, 0, 0, "protected"),
        ("app/.nycrc.json", None, 0, 0, 0, "protected"),
        ("app/package.json", None, 0, 0, 0, "protected"),
        ("app/.husky/pre-push", None, 0, 0, 0, "protected"),
        ("app/lefthook.yml", None, 0, 0, 0, "protected"),
        ("tools/eslint-rules/check.js", None, 0, 0, 0, "protected"),
        ("new.js", Some(".githooks/pre-commit"), 0, 0, 0, "protected"),
        ("tools/eslint-local-rules.js", None, 0, 0, 0, "protected"),
        (
            "packages/tools/eslint/rules/local.mjs",
            None,
            0,
            0,
            0,
            "protected",
        ),
        ("app/.oxfmtrc.json", None, 0, 0, 0, "protected"),
        ("app/.huskyrc.json", None, 0, 0, 0, "protected"),
        ("src/prettier.ts", None, 0, 0, 0, ""),
        ("src/rules/business.ts", None, 0, 0, 0, ""),
        ("vendor/x", Some("x"), 0, 0, 0, "protected"),
        (".blackhole/ledger", None, 2, 1, 1, ""),
        (".blackhole/ledger", None, 1, 2, 2, "findings protected"),
        (".blackhole/objections", None, 0, 0, 0, "protected"),
        ("b", Some("a"), 1, 2, 2, "findings protected"),
        ("b", Some("a"), 2, 2, 2, ""),
        ("x", None, 1, 1, 0, "ledger"),
        ("x", None, 0, 0, 1, "findings"),
    ] {
        let entry = Change {
            kind: from.map_or(Kind::Edited, |old| Kind::Moved(old.into())),
            path: path.into(),
        };
        let counts = |p: &str, n| {
            if n == 0 {
                Counts::new()
            } else {
                Counts::from([(("core/no-suppression".into(), p.into()), n)])
            }
        };
        let dest = if from.is_some() { path } else { "x" };
        let head = counts(dest, new);
        let base = counts(from.unwrap_or(dest), old);
        let mut changes = vec![entry];
        if from.is_some() && new > 0 {
            changes.push(Change {
                kind: Kind::Edited,
                path: ".blackhole/ledger".into(),
            });
        }
        let findings = (0..found)
            .map(|_| Finding {
                rule: "core/no-suppression",
                path: dest.into(),
                line: 1,
                col: 1,
                note: None,
            })
            .collect::<Vec<_>>();
        let result = judge(&Inputs {
            approved: false,
            today: "2026-11-18".parse().unwrap(),
            excludes: &excludes,
            changes: &changes,
            base_ledger: &base,
            head_ledger: Ok(&head),
            base_rows: &Rows::new(),
            head_rows: Ok(&Rows::new()),
            findings: &findings,
        })
        .unwrap();
        assert_eq!(
            result.gates.into_iter().collect::<Vec<_>>().join(" "),
            expected,
            "{path}"
        );
    }
}

#[test]
fn a_move_git_reports_as_two_changes_is_protected() {
    let excludes = GlobSetBuilder::new()
        .add(Glob::new("vendor/**").unwrap())
        .build()
        .unwrap();
    for (old, added, deleted, expected) in [
        (None, "vendor/x", Some("a"), "protected"),
        (None, "vendor/x", None, ""),
        (None, "vendor/x", Some("vendor/y"), ""),
        (None, "x", Some("a"), ""),
        (Some("vendor/old"), "vendor/x", Some("a"), "protected"),
        (Some("vendor/old"), "vendor/x", None, ""),
    ] {
        let mut changes = vec![Change {
            kind: old.map_or(Kind::Added, |old: &str| Kind::Moved(old.into())),
            path: added.into(),
        }];
        changes.extend(deleted.map(|path| Change {
            kind: Kind::Deleted,
            path: path.into(),
        }));
        let result = judge(&Inputs {
            approved: false,
            today: "2026-11-18".parse().unwrap(),
            excludes: &excludes,
            changes: &changes,
            base_ledger: &Counts::new(),
            head_ledger: Ok(&Counts::new()),
            base_rows: &Rows::new(),
            head_rows: Ok(&Rows::new()),
            findings: &[],
        })
        .unwrap();
        assert_eq!(
            result.gates.into_iter().collect::<Vec<_>>().join(" "),
            expected,
            "{old:?} {added} {deleted:?}"
        );
    }
}

#[test]
fn objection_transitions() {
    let today = "2026-11-18".parse().unwrap();
    let empty = Counts::new();
    let excludes = GlobSetBuilder::new().build().unwrap();
    let findings = [Finding {
        rule: "core/no-suppression",
        path: "x".into(),
        line: 1,
        col: 1,
        note: None,
    }];
    for (approval, old, new, gates) in [
        (false, "", "-", "findings protected"),
        (true, "", "-", ""),
        (false, "-", "-", ""),
        (true, "-", "2026-11-18", ""),
        (true, "", "2026-11-18", "findings objections"),
        (true, "2026-11-17", "2026-11-18", "findings objections"),
        (false, "-", "", "findings protected"),
    ] {
        // Git lists the file only when it changed.
        let change = (old != new).then(|| Change {
            kind: Kind::Edited,
            path: ".blackhole/objections".into(),
        });
        let rows = |renew: &str| {
            let text = if renew.is_empty() {
                String::new()
            } else {
                format!("core/no-suppression\tx\t1\t2026-11-01\t{renew}\treason\n")
            };
            blackhole_check::objections::parse(text.as_bytes(), today).unwrap()
        };
        let result = judge(&Inputs {
            approved: approval,
            today,
            excludes: &excludes,
            changes: change.as_slice(),
            base_ledger: &empty,
            head_ledger: Ok(&empty),
            base_rows: &rows(old),
            head_rows: Ok(&rows(new)),
            findings: &findings,
        })
        .unwrap();
        assert_eq!(
            result.gates.into_iter().collect::<Vec<_>>().join(" "),
            gates
        );
    }
}

#[test]
fn rejects_approval_typos() {
    for json in [
        r#"{"authors":["a"],"reviews":[],"typo":1}"#,
        r#"{"author":"a","reviews":[]}"#,
        r#"{"authors":["a"],"reviews":[{"login":"b","state":"APPROVED","commit":"head","permission":"write","typo":0}]}"#,
    ] {
        assert!(serde_json::from_str::<Approvals>(json).is_err());
    }
}
