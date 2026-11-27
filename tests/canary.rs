pub mod common;
#[path = "common/gh.rs"]
mod gh;
use common::git;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
use tempfile::{TempDir, tempdir};

const CHECKS: &str = "repos/owner/repo/commits/SHA/check-runs?check_name=blackhole&per_page=100";
const OPEN_PRS: &str = "repos/owner/repo/pulls?head=owner:blackhole-canary-N&state=open";
const REF: &str = "repos/owner/repo/git/refs/heads/blackhole-canary-N";
const REF_LOOKUP: &str = "repos/owner/repo/git/ref/heads/blackhole-canary-N";
const LOGS: &str = "repos/owner/repo/actions/jobs/99/logs";
const LINES: [&str; 6] = [
    "ledger: raise a ledger allowance",
    "protected: edit blackhole.toml",
    "objections: file an objection with an invalid shape",
    "tests: delete src/app.test.ts",
    "findings: add a finding in blackhole-canary-finding.py",
    "protected: edit .github/workflows/blackhole.yml",
];
const RED: &str = "2026-11-26T10:00:00.0000000Z blackhole ci: red (findings, ledger, objections, protected, tests)\n";

struct Adopter {
    _dir: TempDir,
    remote: std::path::PathBuf,
    root: std::path::PathBuf,
    tip: String,
    fake: gh::FakeGh,
}

// An adopted checkout whose origin names owner/repo on GitHub and leads to a local bare repository instead.
fn adopted() -> Adopter {
    adopted_with(|_| {})
}

fn adopted_with(prepare: impl FnOnce(&Path)) -> Adopter {
    let dir = tempdir().unwrap();
    let remote = dir.path().join("remote.git");
    let root = dir.path().join("checkout");
    fs::create_dir(&root).unwrap();
    fs::create_dir(dir.path().join("home")).unwrap();
    git(dir.path(), &["init", "-q", "--bare", "remote.git"]).unwrap();
    git(&root, &["init", "-q", "-b", "main"]).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("src/app.test.ts"),
        "test(\"adds\", () => {\n  expect(1 + 1).toBe(2);\n});\n",
    )
    .unwrap();
    prepare(&root);
    let fake = gh::FakeGh::new();
    let init = run(&root, &fake, &["init"]);
    assert_eq!(init.status.code(), Some(1), "{init:?}");
    let url = "https://github.com/owner/repo.git";
    git(&root, &["remote", "add", "origin", url]).unwrap();
    let rewrite = format!("url.{}.insteadOf", remote.display());
    git(&root, &["config", &rewrite, url]).unwrap();
    git(&root, &["add", "-A"]).unwrap();
    git(&root, &["commit", "-q", "-m", "adopt blackhole"]).unwrap();
    git(&root, &["push", "-q", "origin", "main"]).unwrap();
    let tip = git(&root, &["rev-parse", "HEAD"]).unwrap();
    fake.respond(
        "GET",
        "repos/owner/repo",
        200,
        &[],
        br#"{"default_branch":"main"}"#,
    );
    fake.respond(
        "POST",
        "repos/owner/repo/pulls",
        201,
        &[],
        br#"{"number":12}"#,
    );
    fake.respond("PATCH", "repos/owner/repo/pulls/12", 200, &[], b"{}");
    fake.respond("GET", OPEN_PRS, 200, &[], br#"[{"number":12}]"#);
    fake.respond("DELETE", REF, 204, &[], b"");
    Adopter {
        _dir: dir,
        remote,
        root,
        tip,
        fake,
    }
}

fn command(root: &Path, fake: &gh::FakeGh, args: &[&str]) -> Command {
    let mut command = common::command(env!("CARGO_BIN_EXE_blackhole"));
    command
        .current_dir(root)
        .args(args)
        .env("PATH", fake.path())
        // The binary drops GIT_* variables, so a home of its own keeps the user's git configuration and hooks out.
        .env("HOME", root.with_file_name("home"))
        .env("XDG_CONFIG_HOME", root.with_file_name("home"));
    command
}

fn run(root: &Path, fake: &gh::FakeGh, args: &[&str]) -> Output {
    command(root, fake, args).output().unwrap()
}

fn finished(adopter: &Adopter, conclusion: &str, log: &str) {
    let run = format!(
        r#"{{"check_runs":[{{"id":99,"status":"completed","conclusion":"{conclusion}","app":{{"id":15368}}}}]}}"#
    );
    adopter
        .fake
        .respond("GET", CHECKS, 200, &[], run.as_bytes());
    adopter.fake.respond("GET", LOGS, 200, &[], log.as_bytes());
}

fn canary(adopter: &Adopter, args: &[&str]) -> (String, i32) {
    let out = run(&adopter.root, &adopter.fake, &[&["canary"], args].concat());
    assert_eq!(out.stderr, b"", "{}", String::from_utf8_lossy(&out.stderr));
    (
        String::from_utf8(out.stdout).unwrap(),
        out.status.code().unwrap(),
    )
}

// Every path must close the pull request and then delete the branch it pushed.
fn assert_cleaned(fake: &gh::FakeGh) -> String {
    let calls = fake.calls();
    let close = calls.iter().position(|call| {
        call.ends_with("--method PATCH -f state=closed repos/owner/repo/pulls/12")
    });
    let delete = calls.iter().position(|call| {
        call.contains("--method DELETE -i repos/owner/repo/git/refs/heads/blackhole-canary-")
    });
    assert!(close.is_some() && close < delete, "{calls:#?}");
    let call = &calls[delete.unwrap()];
    call.rsplit('/').next().unwrap().to_owned()
}

fn expect(lines: &[String], branch: &str) -> String {
    format!(
        "{}\nClosed the canary pull request and deleted branch {branch}.\n",
        lines.join("\n")
    )
}

fn ci(adopter: &Adopter, head: &str) -> String {
    let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(&adopter.remote)
        .args(["ci", "--base", &adopter.tip, "--head", head])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn every_forgery_is_stopped_by_the_real_judge() {
    let adopter = adopted();
    let log = format!(
        "protected: .blackhole/ledger: a person has to approve this change\nprotected: .github/workflows/blackhole.yml: a person has to approve this change\nprotected: blackhole.toml: a person has to approve this change\n{RED}"
    );
    finished(&adopter, "failure", &log);
    let (out, code) = canary(&adopter, &[]);
    let branch = assert_cleaned(&adopter.fake);
    let stopped: Vec<_> = LINES
        .iter()
        .map(|line| format!("{line}: stopped"))
        .collect();
    assert_eq!(out, expect(&stopped, &branch));
    assert_eq!(code, 0);
    let pr = adopter
        .fake
        .calls()
        .into_iter()
        .find(|call| call.contains("--method POST"))
        .unwrap();
    assert!(
        pr.contains(&format!("-f head={branch} -f base=main")),
        "{pr}"
    );
    assert!(pr.contains("-F draft=true"), "{pr}");

    // The judge itself, run on the pushed commits, names every gate and every protected forgery.
    let head = git(&adopter.remote, &["rev-parse", &branch]).unwrap();
    let verdict = ci(&adopter, &head);
    assert!(
        verdict.ends_with(RED.split_once(' ').unwrap().1),
        "{verdict}"
    );
    for path in ["blackhole.toml", ".github/workflows/blackhole.yml"] {
        assert!(
            verdict.contains(&format!("protected: {path}: ")),
            "{verdict}"
        );
    }
    let checked = adopter
        .fake
        .calls()
        .into_iter()
        .find(|call| call.contains("check-runs"))
        .unwrap();
    assert!(checked.contains(&head), "{checked}");
}

#[test]
fn a_refused_workflow_push_counts_as_stopped() {
    let adopter = adopted();
    let hook = adopter.remote.join("hooks/pre-receive");
    fs::write(
        &hook,
        "#!/bin/sh\nwhile read -r old new ref; do\n  if git diff --name-only refs/heads/main \"$new\" | grep -q '^.github/workflows/'; then\n    echo 'refusing to allow an OAuth App to create or update workflow `.github/workflows/blackhole.yml` without `workflow` scope' >&2\n    exit 1\n  fi\ndone\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let log = format!("protected: blackhole.toml: a person has to approve this change\n{RED}");
    finished(&adopter, "failure", &log);
    let (out, code) = canary(&adopter, &[]);
    let branch = assert_cleaned(&adopter.fake);
    let mut lines: Vec<_> = LINES
        .iter()
        .map(|line| format!("{line}: stopped"))
        .collect();
    lines[5] = format!(
        "{}: stopped by GitHub, which refused the push because gh's credential cannot change workflows",
        LINES[5]
    );
    assert_eq!(out, expect(&lines, &branch));
    assert_eq!(code, 0);
    let head = git(&adopter.remote, &["rev-parse", &branch]).unwrap();
    let changed = git(
        &adopter.remote,
        &["diff", "--name-only", &adopter.tip, &head],
    )
    .unwrap();
    assert!(!changed.contains(".github"), "{changed}");
}

#[test]
fn a_forgery_the_check_does_not_name_got_through() {
    let adopter = adopted();
    let log = "protected: blackhole.toml: a person has to approve this change\nblackhole ci: red (findings, ledger, protected)\n";
    finished(&adopter, "failure", log);
    let (out, code) = canary(&adopter, &[]);
    let branch = assert_cleaned(&adopter.fake);
    let verdicts = [
        "stopped",
        "stopped",
        "not stopped",
        "not stopped",
        "stopped",
        "not stopped",
    ];
    let lines: Vec<_> = LINES
        .iter()
        .zip(verdicts)
        .map(|(line, verdict)| format!("{line}: {verdict}"))
        .collect();
    assert_eq!(out, expect(&lines, &branch));
    assert_eq!(code, 1);

    let passing = adopted();
    finished(&passing, "success", RED);
    let (out, code) = canary(&passing, &[]);
    assert_eq!(code, 1);
    assert_eq!(out.matches(": not stopped\n").count(), 6, "{out}");
    assert_cleaned(&passing.fake);
}

#[test]
fn no_verdict_in_time_is_a_timeout() {
    let adopter = adopted();
    let pending = br#"{"check_runs":[{"id":99,"status":"in_progress","conclusion":null,"app":{"id":15368}}]}"#;
    adopter.fake.respond("GET", CHECKS, 200, &[], pending);
    let (out, code) = canary(&adopter, &["--timeout", "0"]);
    let branch = assert_cleaned(&adopter.fake);
    let head = git(&adopter.remote, &["rev-parse", &branch]).unwrap();
    let mut lines = vec![format!(
        "canary: no finished blackhole check on {head} after 0 seconds (check that the blackhole workflow runs on pull request #12, or rerun with a longer --timeout)"
    )];
    lines.extend(LINES.iter().map(|line| format!("{line}: no verdict")));
    assert_eq!(out, expect(&lines, &branch));
    assert_eq!(code, 75);
}

#[test]
fn an_interrupted_canary_still_cleans_up() {
    let adopter = adopted();
    adopter
        .fake
        .respond("GET", CHECKS, 200, &[], br#"{"check_runs":[]}"#);
    let mut child = command(
        &adopter.root,
        &adopter.fake,
        &["canary", "--timeout", "600"],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let waiting = |predicate: &dyn Fn(&[String]) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !predicate(&adopter.fake.calls()) {
            assert!(Instant::now() < deadline, "{:#?}", adopter.fake.calls());
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    waiting(&|calls| calls.iter().any(|call| call.contains("check-runs")));
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!child.wait().unwrap().success());
    waiting(&|calls| calls.iter().any(|call| call.contains("--method DELETE")));
    assert_cleaned(&adopter.fake);
}

#[test]
fn a_lost_create_response_still_closes_the_pull_request() {
    let adopter = adopted();
    adopter.fake.fail(
        "POST",
        "repos/owner/repo/pulls",
        1,
        "error connecting to api.github.com\n",
    );
    let out = run(&adopter.root, &adopter.fake, &["canary"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stdout, b"");
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "blackhole: cannot read GitHub (error connecting to api.github.com)\n"
    );
    assert_cleaned(&adopter.fake);
}

#[test]
fn a_rejected_first_push_still_cleans_up_an_absent_branch() {
    let adopter = adopted();
    let hook = adopter.remote.join("hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    adopter.fake.respond("GET", OPEN_PRS, 200, &[], b"[]");
    adopter.fake.respond("DELETE", REF, 422, &[], b"{}");
    adopter.fake.respond("GET", REF_LOOKUP, 404, &[], b"{}");
    let out = run(&adopter.root, &adopter.fake, &["canary"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stdout, b"");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("cleanup failed"), "{stderr}");
    let calls = adopter.fake.calls();
    assert!(
        calls
            .iter()
            .any(|call| call.contains("--method GET -i repos/owner/repo/git/ref/heads/")),
        "{calls:#?}"
    );
}

#[test]
fn a_422_delete_requires_confirming_the_ref_is_absent() {
    let adopter = adopted();
    finished(&adopter, "failure", RED);
    adopter.fake.respond("DELETE", REF, 422, &[], b"{}");
    adopter.fake.respond("GET", REF_LOOKUP, 200, &[], b"{}");

    let out = run(&adopter.root, &adopter.fake, &["canary"]);
    let branch = assert_cleaned(&adopter.fake);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&format!("delete branch {branch} by hand")),
        "{out:?}"
    );
}

#[test]
fn a_closed_stdout_cannot_hide_the_cleanup_failure() {
    let adopter = adopted();
    finished(&adopter, "failure", RED);
    adopter.fake.respond("DELETE", REF, 403, &[], b"{}");
    let mut child = command(&adopter.root, &adopter.fake, &["canary"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let branch = assert_cleaned(&adopter.fake);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&format!("delete branch {branch} by hand")),
        "{out:?}"
    );
}

#[test]
fn a_push_and_cleanup_failure_emits_one_escaped_line() {
    let adopter = adopted();
    let hook = adopter.remote.join("hooks/pre-receive");
    fs::write(
        &hook,
        "#!/bin/sh\nprintf 'first\\033line\\nsecond line\\n' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    adopter.fake.respond("GET", OPEN_PRS, 200, &[], b"[]");
    adopter
        .fake
        .fail("DELETE", REF, 1, "raw cleanup error\nmore details\n");
    let out = run(&adopter.root, &adopter.fake, &["canary"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stdout, b"");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("remote: first"), "{stderr:?}");
    assert!(
        !stderr.trim_end().chars().any(char::is_control),
        "{stderr:?}"
    );
    assert!(stderr.contains("by hand"), "{stderr}");
}

fn assert_no_verdict(conclusion: &str, log: &str) {
    let adopter = adopted();
    finished(&adopter, conclusion, log);
    let (out, code) = canary(&adopter, &[]);
    assert_eq!(code, 75, "{out}");
    for line in LINES {
        assert!(
            out.lines().any(|row| row == format!("{line}: no verdict")),
            "{out}"
        );
    }
    assert_cleaned(&adopter.fake);
}

#[test]
fn a_cancelled_check_has_no_verdict() {
    assert_no_verdict("cancelled", RED);
}

#[test]
fn a_job_that_fails_before_the_judge_has_no_verdict() {
    assert_no_verdict(
        "failure",
        "2026-11-26T10:00:00.0000000Z setup-node failed\n",
    );
}

#[test]
fn a_test_shaped_directory_is_skipped_when_choosing_a_test_to_delete() {
    let adopter = adopted_with(|root| {
        let directory = root.join("src/aaa.test.ts");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join(".keep"), "").unwrap();
    });
    finished(&adopter, "failure", RED);
    let (out, _) = canary(&adopter, &[]);
    let branch = assert_cleaned(&adopter.fake);
    assert!(
        out.contains("tests: delete src/app.test.ts: stopped\n"),
        "{out}"
    );
    let head = git(&adopter.remote, &["rev-parse", &branch]).unwrap();
    let verdict = ci(&adopter, &head);
    assert!(verdict.contains("src/app.test.ts"), "{verdict}");
}
