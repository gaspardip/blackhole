use super::*;
use crate::engine::{Family, Gate, Rule, Run};
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
    let output = crate::git::command()
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.org",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
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
        ".blackhole/ledger",
        format!("{HEADER}test/advisory\t2\tadvice.txt\ntest/advisory\t1\tgone.txt\n"),
    )
    .unwrap();
}

#[test]
fn check_reports_advice_without_failing() {
    in_repo(|| {
        let mut out = Vec::new();
        let red = check(&[], &mut out).unwrap();
        legacy_ledger();
        let mut with_ledger = Vec::new();
        let legacy_red = check(&[], &mut with_ledger).unwrap();
        assert_eq!(
            (
                red,
                from_utf8(&out).unwrap(),
                legacy_red,
                from_utf8(&with_ledger).unwrap()
            ),
            (false, FINDING, false, FINDING)
        );
    });
}

#[test]
fn ci_reports_advice_without_failing() {
    in_repo(|| {
        git(&["add", "-A"]);
        git(&["commit", "-qm", "advice"]);
        let mut out = Vec::new();
        let red = ci::run("HEAD", "HEAD", None, None, &mut out).unwrap();
        legacy_ledger();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "legacy ledger"]);
        let mut with_ledger = Vec::new();
        let legacy_red = ci::run("HEAD", "HEAD", None, None, &mut with_ledger).unwrap();
        assert_eq!(
            (
                red,
                from_utf8(&out).unwrap(),
                legacy_red,
                from_utf8(&with_ledger).unwrap()
            ),
            (false, FINDING, false, FINDING)
        );
    });
}

#[test]
fn ledger_leaves_new_advice_unrecorded_and_drops_old_allowances() {
    in_repo(|| {
        let root = env::current_dir().unwrap();
        let red = regenerate(&mut io::sink()).unwrap();
        let ledger = root.join(".blackhole/ledger");
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
