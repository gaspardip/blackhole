use super::*;
use crate::engine::{Family, Gate, Rule, Run};
use crate::files::{LEDGER_PATH, OBJECTIONS_PATH};
use std::{process::Command, str::from_utf8};

pub(super) const RULE: Rule = Rule {
    id: "test/advisory",
    family: Family::Tests,
    gate: Gate::Advisory,
    provenance: "Exercises advisory reporting.",
    fix: "Remove the test marker.",
    run: Run::File(|ctx, findings| {
        if ctx.path == "advice.txt" {
            findings.report(0, Some("test marker"));
        }
    }),
};

const HEADER: &str = "blackhole-ledger 1\n";
const FINDING: &str = "advice.txt:1:1 test/advisory [advisory] test marker\n\
                       test/advisory Remove the test marker.\n";

fn git(args: &[&str]) {
    crate::git::output(
        crate::git::command()
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.org",
            ])
            .args(args),
    )
    .unwrap();
}

fn in_repo(test: impl FnOnce()) {
    // Check and ledger change the process working directory.
    const CHILD: &str = "BLACKHOLE_ADVISORY_TEST_CHILD";
    if env::var_os(CHILD).is_some() {
        git(&["init", "-q"]);
        fs::write("advice.txt", "marker").unwrap();
        test();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let thread = std::thread::current();
    let out = Command::new(env::current_exe().unwrap())
        .current_dir(dir.path())
        .env(CHILD, "1")
        .args(["--exact", thread.name().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

fn legacy_ledger() {
    fs::create_dir(".blackhole").unwrap();
    fs::write(
        LEDGER_PATH,
        format!("{HEADER}test/advisory\t2\tadvice.txt\ntest/advisory\t1\tgone.txt\n"),
    )
    .unwrap();
}

#[test]
fn check_reports_advice_without_failing() {
    in_repo(|| {
        let assert_reports_advice = || {
            let mut out = Vec::new();
            let red = check(&[], &mut out).unwrap();
            assert_eq!((red, from_utf8(&out).unwrap()), (false, FINDING));
        };
        assert_reports_advice();
        legacy_ledger();
        assert_reports_advice();
    });
}

#[test]
fn ci_reports_advice_without_failing() {
    in_repo(|| {
        git(&["add", "-A"]);
        git(&["commit", "-qm", "advice"]);
        legacy_ledger();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "legacy ledger"]);
        for (base, head) in [("HEAD^", "HEAD^"), ("HEAD", "HEAD"), ("HEAD^", "HEAD")] {
            let mut out = Vec::new();
            let red = ci::run(base, head, None, None, &mut out).unwrap();
            assert_eq!(
                (red, from_utf8(&out).unwrap()),
                (false, FINDING),
                "{base} to {head}"
            );
        }
    });
}

#[test]
fn ledger_leaves_new_advice_unrecorded_and_drops_old_allowances() {
    in_repo(|| {
        let root = env::current_dir().unwrap();
        let red = regenerate(&mut io::sink()).unwrap();
        let ledger = root.join(LEDGER_PATH);
        assert!(!ledger.exists());
        legacy_ledger();
        let legacy_red = regenerate(&mut io::sink()).unwrap();
        assert_eq!(
            (
                red,
                legacy_red,
                fs::read_to_string(ledger).unwrap().as_str()
            ),
            (false, false, HEADER)
        );
    });
}

#[test]
fn advisory_rules_take_no_new_objections_and_ignore_old_ones() {
    in_repo(|| {
        let object_error =
            object(RULE.id.into(), "advice.txt".into(), Some("reason"), false).unwrap_err();
        assert_eq!(object_error.to_string(), "rule cannot be objected to");
        // A row left from when the rule was blocking must not break any command.
        fs::create_dir(".blackhole").unwrap();
        fs::write(
            OBJECTIONS_PATH,
            "test/advisory\tadvice.txt\t1\t2026-11-18\t-\treason\n",
        )
        .unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "legacy objection"]);
        let mut checked = Vec::new();
        let check_red = check(&[], &mut checked).unwrap();
        let mut judged = Vec::new();
        let ci_red = ci::run("HEAD", "HEAD", None, None, &mut judged).unwrap();
        assert_eq!(
            (
                check_red,
                from_utf8(&checked).unwrap(),
                ci_red,
                from_utf8(&judged).unwrap()
            ),
            (false, FINDING, false, FINDING)
        );
    });
}
