#[path = "common/cli.rs"]
mod cli;
mod common;

use cli::{check, repo, run};
use common::command;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
    thread,
    time::Duration,
};
use tempfile::tempdir;

fn git(dir: &Path, args: &[&str]) {
    let out = git_output(dir, args);
    assert!(out.status.success(), "{out:?}");
}

fn git_output(dir: &Path, args: &[&str]) -> Output {
    command("git").current_dir(dir).args(args).output().unwrap()
}

fn commit(root: &Path) {
    git(root, &["add", "-A"]);
    let identity = [
        "-c",
        "user.name=fixture",
        "-c",
        "user.email=fixture@example.org",
    ];
    git(root, &[&identity[..], &["commit", "-qm", "tree"]].concat());
}

fn names(dir: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

#[test]
fn check_escapes_newlines_in_finding_paths() {
    let dir = repo();
    fs::write(dir.path().join("line\nbreak.py"), "# noqa").unwrap();
    let text = String::from_utf8(check(dir.path(), &[], 1).stdout).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(text.starts_with(r"line\nbreak.py:1:1 "));
}

#[test]
fn clean_selection_is_silent() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join(".gitignore"), "ignored.py\n").unwrap();
    fs::write(root.join("ignored.py"), "#noqa").unwrap();
    fs::write(root.join("deleted.py"), "#noqa").unwrap();
    git(root, &["add", "deleted.py"]);
    fs::remove_file(root.join("deleted.py")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::create_dir(root.join("app")).unwrap();
        symlink("app", root.join("linked")).unwrap();
        symlink("nowhere", root.join("dangling.py")).unwrap();
    }
    assert_eq!(check(root, &[], 0).stdout, b"");
}

#[cfg(unix)]
#[test]
fn a_linked_file_is_checked_under_its_own_name() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("payload.txt"), "#noqa").unwrap();
    std::os::unix::fs::symlink("payload.txt", root.join("alias.py")).unwrap();
    let found = check(root, &[], 1).stdout;
    assert!(found.starts_with(b"alias.py:1:1 core/no-suppression\n"));
    assert_eq!(check(root, &["alias.py"], 1).stdout, found);
    assert_eq!(check(root, &["payload.txt"], 0).stdout, b"");
}

#[cfg(unix)]
#[test]
fn a_path_is_resolved_the_way_the_filesystem_does() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir_all(root.join("real/sub")).unwrap();
    fs::write(root.join("real/clean.py"), "#noqa").unwrap();
    fs::write(root.join("clean.py"), "").unwrap();
    std::os::unix::fs::symlink("real/sub", root.join("redirect")).unwrap();
    let found = check(root, &["redirect/../clean.py"], 1).stdout;
    assert!(found.starts_with(b"real/clean.py:1:1 core/no-suppression\n"));
}

#[test]
fn git_variables_cannot_point_at_another_repository() {
    let dir = repo();
    let root = dir.path();
    let other = repo();
    fs::write(root.join("bad.py"), "#noqa").unwrap();
    let excludes = other.path().join("excludes");
    fs::write(&excludes, "bad.py\n").unwrap();
    for (name, value) in [
        ("GIT_COMMON_DIR", other.path().join(".git")),
        ("GIT_DIR", other.path().join(".git")),
        ("GIT_WORK_TREE", other.path().to_owned()),
        ("GIT_INDEX_FILE", other.path().join("index")),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .env(name, value)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.excludesFile")
            .env("GIT_CONFIG_VALUE_0", &excludes)
            .arg("check")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{name}: {out:?}");
        assert!(out.stdout.starts_with(b"bad.py:1:1 "), "{name}");
    }
}

#[test]
fn untracked_findings_and_single_fix() {
    let dir = repo();
    fs::write(dir.path().join("bad.py"), b"\xff #noqa\r\n#noqa").unwrap();
    let out = check(dir.path(), &[], 1);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "bad.py:1:3 core/no-suppression\nbad.py:2:1 core/no-suppression\ncore/no-suppression Fix what the tool reports instead of silencing it. If the tool is wrong, tell a person.\n"
    );
}

#[test]
fn an_ignore_rule_hides_an_untracked_file_and_never_a_tracked_one() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "#noqa").unwrap();
    fs::create_dir(root.join(".git/info")).unwrap();
    fs::write(root.join(".git/info/exclude"), "a.py\n").unwrap();
    assert_eq!(check(root, &[], 0).stdout, b"");
    check(root, &["a.py"], 2);
    git(root, &["add", "-f", "a.py"]);
    check(root, &[], 1);
    check(root, &["a.py"], 1);
}

#[test]
fn path_filters_use_directory_components() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join("app")).unwrap();
    fs::write(root.join("app/good.py"), "").unwrap();
    fs::write(root.join("apple.py"), "#noqa").unwrap();
    assert_eq!(check(root, &["app"], 0).stdout, b"");
    assert_eq!(check(root, &["app/good.py"], 0).stdout, b"");
    check(root, &["app", "missing"], 2);
}

#[test]
fn outside_repository() {
    check(tempdir().unwrap().path(), &[], 2);
}

#[cfg(unix)]
#[test]
fn invalid_path_is_an_error() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let dir = repo();
    fs::write(dir.path().join(OsStr::from_bytes(b"bad\xff")), "").unwrap();
    let message = check(dir.path(), &[], 2).stderr;
    assert_eq!(message, b"blackhole: path is not valid UTF-8: bad\\xff\n");
}

const HEADER: &str = "blackhole-ledger 1\n";
const RULE: &str = "core/no-suppression";
const MISMATCH: &[u8] = b"blackhole: checkout root mismatch\n";

fn baseline(dir: &Path, rows: &[(&str, usize)]) -> Vec<u8> {
    fs::create_dir_all(dir.join(".blackhole")).unwrap();
    let mut data = HEADER.to_owned();
    for (path, n) in rows {
        use std::fmt::Write;
        writeln!(data, "{RULE}\t{n}\t{path}").unwrap();
    }
    fs::write(dir.join(".blackhole/ledger"), &data).unwrap();
    data.into_bytes()
}

fn regenerate(dir: &Path, code: i32) -> Vec<u8> {
    run(dir, &["ledger"], code).stdout
}

#[test]
fn ledger_exact_and_over() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "#noqa\n#noqa").unwrap();
    let expected = check(root, &[], 1).stdout;
    baseline(root, &[("a.py", 2)]);
    assert_eq!(check(root, &[], 0).stdout, b"");
    let before = baseline(root, &[("a.py", 1)]);
    assert_eq!(check(root, &[], 1).stdout, expected);
    assert_eq!(regenerate(root, 1), expected);
    assert_eq!(fs::read(root.join(".blackhole/ledger")).unwrap(), before);
}

#[test]
fn stale_and_deleted_entries_shrink() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "#noqa").unwrap();
    fs::write(root.join("b.py"), "").unwrap();
    baseline(root, &[("a.py", 2), ("gone.py", 1)]);
    let stale = format!(
        "a.py {RULE}: ledger allows 2, found 1; run blackhole ledger\n\
         gone.py {RULE}: ledger allows 1, found 0; run blackhole ledger\n"
    );
    assert_eq!(check(root, &[], 1).stdout, stale.as_bytes());
    assert_eq!(check(root, &["b.py"], 0).stdout, b"");
    let selected = format!("a.py {RULE}: ledger allows 2, found 1; run blackhole ledger\n");
    assert_eq!(check(root, &["a.py"], 1).stdout, selected.as_bytes());
    assert_eq!(regenerate(root, 0), b"");
    assert_eq!(
        fs::read(root.join(".blackhole/ledger")).unwrap(),
        format!("{HEADER}{RULE}\t1\ta.py\n").as_bytes()
    );
    assert_eq!(check(root, &[], 0).stdout, b"");
}

#[test]
fn a_selection_keeps_the_rows_of_files_that_are_gone() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/clean.py"), "").unwrap();
    fs::create_dir(root.join("empty")).unwrap();
    fs::write(root.join("other.py"), "").unwrap();
    baseline(
        root,
        &[
            ("empty/gone.py", 1),
            ("gone.py", 1),
            ("gone/deep/file.py", 1),
            ("sub/gone.py", 1),
            ("subway/gone.py", 1),
        ],
    );
    let stale = format!("sub/gone.py {RULE}: ledger allows 1, found 0; run blackhole ledger\n");
    assert_eq!(check(root, &["sub"], 1).stdout, stale.as_bytes());
    assert_eq!(check(&root.join("sub"), &["."], 1).stdout, stale.as_bytes());
    assert_eq!(check(root, &["other.py"], 0).stdout, b"");
    let stale = format!("empty/gone.py {RULE}: ledger allows 1, found 0; run blackhole ledger\n");
    assert_eq!(check(root, &["empty"], 1).stdout, stale.as_bytes());
    let stale =
        format!("gone/deep/file.py {RULE}: ledger allows 1, found 0; run blackhole ledger\n");
    for path in ["gone", "gone/deep/file.py"] {
        assert_eq!(check(root, &[path], 1).stdout, stale.as_bytes(), "{path}");
    }
    for path in ["nowhere", "nowhere/deep.py", "gone/../other.py", ".."] {
        check(root, &[path], 2);
    }
}

#[test]
fn absent_baseline() {
    let dir = repo();
    let root = dir.path();
    assert_eq!(regenerate(root, 0), b"");
    for path in ["a\tb.py", "a\nb.py"] {
        fs::write(root.join(path), "#noqa").unwrap();
    }
    let found = regenerate(root, 1);
    assert_eq!(found, check(root, &[], 1).stdout);
    let found = String::from_utf8(found).unwrap();
    for line in [
        format!("a\tb.py:1:1 {RULE}\n"),
        format!("a\nb.py:1:1 {RULE}\n"),
    ] {
        assert!(found.contains(&line), "{found:?}");
    }
    assert_eq!(found.matches(&format!("\n{RULE} Fix ")).count(), 1);
    assert!(!root.join(".blackhole/ledger").exists());
}

#[test]
fn over_blocks_the_shrink() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "#noqa\n#noqa").unwrap();
    let before = baseline(root, &[("a.py", 1), ("gone.py", 1)]);
    regenerate(root, 1);
    assert_eq!(fs::read(root.join(".blackhole/ledger")).unwrap(), before);
}

#[test]
fn regeneration_waits_for_the_lock() {
    let other = repo();
    // The caller's git variables must not move the lock away from the ledger it guards.
    for case in ["plain", "variables"] {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("a.py"), "#noqa").unwrap();
        let before = baseline(root, &[("a.py", 2)]);
        let lock = fs::File::open(root).unwrap();
        lock.lock().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_blackhole"));
        command.current_dir(root).arg("ledger");
        if case == "variables" {
            command
                .env("GIT_DIR", other.path().join(".git"))
                .env("GIT_WORK_TREE", root);
        }
        let mut child = command.spawn().unwrap();
        thread::sleep(Duration::from_millis(500));
        assert!(child.try_wait().unwrap().is_none(), "{case}");
        assert_eq!(fs::read(root.join(".blackhole/ledger")).unwrap(), before);
        lock.unlock().unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            fs::read(root.join(".blackhole/ledger")).unwrap(),
            format!("{HEADER}{RULE}\t1\ta.py\n").as_bytes()
        );
    }
}

#[test]
fn the_root_is_found_once() {
    let dir = repo();
    let root = dir.path();
    let sub = root.join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("a.py"), "#noqa").unwrap();
    baseline(root, &[("sub/a.py", 2)]);
    let inner = baseline(&sub, &[("a.py", 2)]);
    let lock = fs::File::open(root).unwrap();
    lock.lock().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(&sub)
        .arg("ledger")
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(500));
    assert!(child.try_wait().unwrap().is_none());
    // A repository that appears under the tree while the writer waits must not become its root.
    git(&sub, &["init", "-q"]);
    lock.unlock().unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read(sub.join(".blackhole/ledger")).unwrap(), inner);
    assert_eq!(
        fs::read(root.join(".blackhole/ledger")).unwrap(),
        HEADER.as_bytes()
    );
}

#[cfg(unix)]
#[test]
fn the_lock_is_not_a_file() {
    let dir = tempdir().unwrap();
    let (meta, root) = (dir.path().join("meta"), dir.path().join("tree"));
    let at = |path: &Path| path.to_str().unwrap().to_owned();
    git(
        dir.path(),
        &["init", "-q", "--separate-git-dir", &at(&meta), &at(&root)],
    );
    fs::write(root.join("a.py"), "").unwrap();
    let victim = dir.path().join("victim");
    fs::write(&victim, "kept").unwrap();
    let absent = dir.path().join("absent");
    // A file of the old lock's name, wherever it points, is no part of the lock.
    std::os::unix::fs::symlink(&absent, root.join("blackhole.lock")).unwrap();
    regenerate(&root, 0);
    assert!(!meta.join("blackhole.lock").try_exists().unwrap());
    assert!(!absent.try_exists().unwrap());
    fs::remove_file(root.join("blackhole.lock")).unwrap();
    std::os::unix::fs::symlink(&victim, meta.join("blackhole.lock")).unwrap();
    regenerate(&root, 0);
    assert_eq!(fs::read(&victim).unwrap(), b"kept");
    assert_eq!(names(&root), [".git", "a.py"]);
}

#[cfg(unix)]
type Outside =
    std::collections::BTreeMap<std::path::PathBuf, (Vec<u8>, Option<std::time::SystemTime>)>;

#[cfg(unix)]
fn shared_index(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.as_encoded_bytes().starts_with(b"sharedindex."))
}

#[cfg(unix)]
fn outside(dir: &Path, root: &Path, seen: &mut Outside) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path == root {
            continue;
        }
        if path.is_dir() {
            outside(&path, root, seen);
            seen.insert(path, (Vec::new(), None));
            continue;
        }
        // Git refreshes the shared index's timestamps each time it reads a split index.
        let modified =
            (!shared_index(&path)).then(|| fs::metadata(&path).unwrap().modified().unwrap());
        seen.insert(path.clone(), (fs::read(&path).unwrap(), modified));
    }
}

#[cfg(unix)]
#[test]
fn nothing_outside_the_checkout_changes_but_the_shared_index_times() {
    let dir = tempdir().unwrap();
    let (meta, root) = (dir.path().join("meta"), dir.path().join("tree"));
    let at = |path: &Path| path.to_str().unwrap().to_owned();
    git(
        dir.path(),
        &["init", "-q", "--separate-git-dir", &at(&meta), &at(&root)],
    );
    fs::write(root.join("a.py"), "#noqa").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["update-index", "--split-index"]);
    fs::write(dir.path().join("victim"), "kept").unwrap();
    let snapshot = || {
        let mut seen = Outside::new();
        outside(dir.path(), &root, &mut seen);
        seen
    };
    let before = snapshot();
    assert!(before.keys().any(|path| shared_index(path)));
    let assert_unchanged = |command| {
        assert!(
            snapshot() == before,
            "{command} changed a file outside the checkout"
        );
    };
    baseline(&root, &[("a.py", 2)]);
    check(&root, &[], 1);
    assert_unchanged("check");

    regenerate(&root, 0);
    assert_eq!(
        fs::read_to_string(root.join(".blackhole/ledger")).unwrap(),
        format!("{HEADER}{RULE}\t1\ta.py\n")
    );
    assert_unchanged("ledger");

    fs::write(root.join("a.py"), "#noqa\n#noqa").unwrap();
    run(&root, &["object", RULE, "a.py", "reason"], 0);
    assert_unchanged("object");
}

#[cfg(unix)]
#[test]
fn a_broken_entry_on_the_way_is_not_a_directory_that_is_gone() {
    let dir = repo();
    let root = dir.path();
    baseline(root, &[("gone/deep/file.py", 1)]);
    let stale =
        format!("gone/deep/file.py {RULE}: ledger allows 1, found 0; run blackhole ledger\n");
    fs::write(root.join("gone"), "").unwrap();
    assert_eq!(
        check(root, &["gone/deep/file.py"], 1).stdout,
        stale.as_bytes()
    );
    fs::remove_file(root.join("gone")).unwrap();
    std::os::unix::fs::symlink("../old-location", root.join("gone")).unwrap();
    check(root, &["gone/deep/file.py"], 2);
}

#[test]
fn a_path_of_many_missing_directories_is_an_error() {
    let dir = repo();
    let root = dir.path();
    check(root, &[&format!("{}f.py", "x/".repeat(60_000))], 2);
    // A name too long to look at is not a directory that is gone, even with a row under it.
    let long = format!("gone/{}f.py", "x/".repeat(60_000));
    baseline(root, &[(&long, 1)]);
    fs::write(root.join("gone"), "").unwrap();
    check(root, &[&long], 2);
}

#[test]
fn invalid_baseline() {
    let dir = repo();
    let root = dir.path();
    baseline(root, &[]);
    let path = root.join(".blackhole/ledger");
    fs::write(&path, b"invalid").unwrap();
    check(root, &[], 2);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    check(root, &[], 2);
}

#[cfg(unix)]
#[test]
fn symlink_baseline() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".blackhole")).unwrap();
    fs::write(root.join("target"), HEADER).unwrap();
    std::os::unix::fs::symlink("../target", root.join(".blackhole/ledger")).unwrap();
    check(root, &[], 2);
}

#[cfg(unix)]
#[test]
fn symlinked_ledger_directory() {
    let dir = repo();
    let root = dir.path();
    let other = repo();
    let outside = baseline(other.path(), &[("a.py", 2)]);
    std::os::unix::fs::symlink(other.path().join(".blackhole"), root.join(".blackhole")).unwrap();
    fs::write(root.join("a.py"), "#noqa").unwrap();
    check(root, &[], 2);
    run(root, &["ledger"], 2);
    assert_eq!(
        fs::read(other.path().join(".blackhole/ledger")).unwrap(),
        outside
    );
}

#[test]
fn root_config_controls_check_and_ledger() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "#noqa").unwrap();
    fs::write(root.join("blackhole.toml"), "exclude = ['*.py']").unwrap();
    assert_eq!(check(root, &[], 0).stdout, b"");
    baseline(root, &[("a.py", 1)]);
    regenerate(root, 0);
    assert_eq!(
        fs::read(root.join(".blackhole/ledger")).unwrap(),
        HEADER.as_bytes()
    );
}

#[test]
fn unknown_config_key_is_an_error() {
    let dir = repo();
    fs::write(dir.path().join("blackhole.toml"), "excludes = []").unwrap();
    check(dir.path(), &[], 2);
}

#[test]
fn lcov_requires_a_repository_relative_path() {
    let dir = repo();
    let config = dir.path().join("blackhole.toml");
    for path in ["", "../report", "/tmp/report", "C:/tmp/report", "D:report"] {
        fs::write(&config, format!("[coverage]\nlcov = {path:?}")).unwrap();
        check(dir.path(), &[], 2);
    }
    fs::write(config, "[coverage]\nlcov = './reports/lcov.info'").unwrap();
    check(dir.path(), &[], 0);
}

#[test]
fn objection_commands_and_shrink() {
    let dir = repo();
    let root = dir.path();
    let args = ["object", RULE, "bad.py", "reason"];
    fs::write(root.join("bad.py"), "").unwrap();
    run(root, &args, 2);
    fs::write(root.join("bad.py"), "#noqa\n#noqa").unwrap();
    baseline(root, &[("bad.py", 1), ("gone.py", 1)]);
    run(root, &["object", RULE, "bad.py", "a\tb"], 2);
    run(root, &args, 0);
    run(root, &args, 2);
    let path = root.join(".blackhole/objections");
    let today = std::process::Command::new("date")
        .args(["-u", "+%F"])
        .output()
        .unwrap()
        .stdout;
    let today = std::str::from_utf8(&today).unwrap().trim();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{RULE}\tbad.py\t1\t{today}\t-\treason\n")
    );
    regenerate(root, 0);
    assert_eq!(
        fs::read_to_string(root.join(".blackhole/ledger")).unwrap(),
        format!("{HEADER}{RULE}\t1\tbad.py\n")
    );
    check(root, &[], 0);
    let renew = ["object", "--renew", RULE, "bad.py"];
    run(root, &renew, 0);
    assert!(String::from_utf8_lossy(&run(root, &renew, 2).stderr).contains("already renewed once"));
    fs::write(&path, format!("{RULE}\tbad.py\t1\t2000-01-01\t-\treason\n")).unwrap();
    check(root, &[], 1);
    fs::write(path, "invalid").unwrap();
    check(root, &[], 2);
}

#[test]
fn ci_uses_commits_and_preserves_dirty_index_and_tree() {
    let dir = repo();
    let root = dir.path();
    run(root, &["ci", "--base", "1234", "--head", "abcd"], 2);
    fs::write(root.join("a.py"), "pass").unwrap();
    commit(root);
    fs::write(root.join("a.py"), "# noqa staged").unwrap();
    git(root, &["add", "."]);
    fs::write(root.join("a.py"), "# noqa unstaged").unwrap();
    let index = fs::read(root.join(".git/index")).unwrap();
    assert_eq!(
        run(root, &["ci", "--base", "HEAD", "--head", "HEAD"], 0).stdout,
        b""
    );
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    let other = repo();
    let out = Command::new(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(root)
        .env("GIT_DIR", other.path().join(".git"))
        .args(["ci", "--base", "HEAD", "--head", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(root.join("a.py")).unwrap(), b"# noqa unstaged");
}

#[test]
fn ci_is_behind_when_head_lacks_base() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "pass").unwrap();
    commit(root);
    fs::write(root.join("b.py"), "pass").unwrap();
    commit(root);
    assert_eq!(
        run(root, &["ci", "--base", "HEAD", "--head", "HEAD~"], 1).stdout,
        b"head does not contain base; update the branch\nblackhole ci: red (behind)\n"
    );
}

#[test]
fn malformed_head_objection_identifies_key_and_discards_all_allowances() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".blackhole")).unwrap();
    let file = root.join(".blackhole/objections");
    let valid = format!("{RULE}\ta.py\t1\t2026-11-01\t-\treason\n");
    fs::write(&file, &valid).unwrap();
    fs::write(root.join("a.py"), "# noqa").unwrap();
    commit(root);
    fs::write(
        file,
        format!("{valid}{RULE}\tb.py\t1\t2026-11-19\t-\treason\n"),
    )
    .unwrap();
    commit(root);
    let out = run(
        root,
        &[
            "ci",
            "--base",
            "HEAD~",
            "--head",
            "HEAD",
            "--today",
            "2026-11-18",
        ],
        1,
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(&format!("objection: {RULE} b.py:")));
    assert!(stdout.ends_with("blackhole ci: red (findings, objections, protected)\n"));
}

#[test]
fn ci_reads_attributes_from_the_base_commit() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "x = 1  # noqa\ny = 2\nz = 3\n").unwrap();
    baseline(root, &[("a.py", 1)]);
    commit(root);
    fs::remove_file(root.join("a.py")).unwrap();
    fs::write(root.join("b.py"), "x = 1  # noqa\r\ny = 2\r\nz = 3\r\n").unwrap();
    baseline(root, &[("b.py", 1)]);
    commit(root);
    let ci = ["ci", "--base", "HEAD~", "--head", "HEAD"];
    run(root, &ci, 0);
    // Git compares a file line by line, and so sees this rename, only while its attributes call it text.
    fs::write(root.join(".gitattributes"), "*.py -diff\n").unwrap();
    run(root, &ci, 0);
}

#[cfg(unix)]
#[test]
fn ci_reads_a_link_the_way_a_checkout_does() {
    use std::os::unix::fs::symlink;
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("clean.py"), "pass").unwrap();
    commit(root);
    fs::create_dir(root.join("real")).unwrap();
    fs::write(root.join("real/payload.txt"), "#noqa").unwrap();
    symlink("real", root.join("dir")).unwrap();
    symlink("dir/../dir/payload.txt", root.join("alias.py")).unwrap();
    symlink("alias.py", root.join("again.py")).unwrap();
    commit(root);
    let ci = ["ci", "--base", "HEAD~", "--head", "HEAD"];
    let out = String::from_utf8(run(root, &ci, 1).stdout).unwrap();
    let found = format!("again.py:1:1 {RULE}\nalias.py:1:1 {RULE}\n");
    assert!(out.starts_with(&found), "{out}");
    // A link the commit cannot resolve could hide a file from the judge.
    for target in [
        "nowhere",
        "../outside",
        "/etc/hostname",
        "loop.py",
        "clean.py/x",
    ] {
        symlink(target, root.join("loop.py")).unwrap();
        commit(root);
        run(root, &ci, 2);
        fs::remove_file(root.join("loop.py")).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn objections_refuse_a_linked_directory() {
    let dir = repo();
    let root = dir.path();
    let other = tempdir().unwrap();
    let outside = other.path().join("objections");
    let row = format!("{RULE}\tbad.py\t1\t2026-01-01\t-\treason\n");
    fs::write(&outside, &row).unwrap();
    std::os::unix::fs::symlink(other.path(), root.join(".blackhole")).unwrap();
    run(root, &["object", "--renew", RULE, "bad.py"], 2);
    assert_eq!(fs::read_to_string(&outside).unwrap(), row);
}

#[test]
fn filing_waits_for_the_lock() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("bad.py"), "#noqa").unwrap();
    let lock = fs::File::open(root).unwrap();
    lock.lock().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(root)
        .args(["object", RULE, "bad.py", "reason"])
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(500));
    assert!(child.try_wait().unwrap().is_none());
    assert!(!root.join(".blackhole/objections").exists());
    lock.unlock().unwrap();
    assert!(child.wait().unwrap().success());
    assert!(root.join(".blackhole/objections").exists());
}

#[test]
fn parse_errors_cannot_be_banked_or_objected_to() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("bad.ts"), "const = ;").unwrap();
    run(root, &["ledger"], 1);
    assert!(!root.join(".blackhole/ledger").exists());
    run(root, &["object", "core/unparseable", "bad.ts", "broken"], 2);
    assert!(!root.join(".blackhole/objections").exists());
    let policy = root.join(".blackhole");
    fs::create_dir_all(&policy).unwrap();
    fs::write(
        policy.join("ledger"),
        "blackhole-ledger 1\ncore/unparseable\t1\tbad.ts\n",
    )
    .unwrap();
    check(root, &[], 2);
    fs::remove_file(policy.join("ledger")).unwrap();
    fs::write(
        policy.join("objections"),
        "core/unparseable\tbad.ts\t1\t2026-01-01\t-\tbroken\n",
    )
    .unwrap();
    check(root, &[], 2);
}

const RUNNER_CASES: &[(&str, &str, &str)] = &[
    (
        "case.test.ts",
        "vi.waitFor(() => expect(calls.length)[matcher](1));",
        "ts/no-unsignaled-poll",
    ),
    (
        "case.test.ts",
        "vi.waitFor(() => expect(calls.current)[matcher](1));",
        "",
    ),
    (
        "case.test.ts",
        "const value = read(); const matcher = expect(value).toBe; test('x', () => { const value = 1; matcher(value); });",
        "",
    ),
    (
        "case.test.ts",
        "const value = 1; const matcher = expect(value).toBe; test('x', () => { const value = 1; matcher(value); });",
        "ts/no-assertionless-tests",
    ),
    (
        "case.test.ts",
        "test('x', () => { assert.equal(read(), 1); });",
        "",
    ),
    (
        "case.test.ts",
        "import assert from 'node:assert/strict'; test('x', () => { assert.equal(read(), 1); });",
        "",
    ),
    (
        "case.test.ts",
        "import {equal as verify} from 'assert'; test('x', () => { verify(read(), 1); });",
        "",
    ),
    (
        "case.test.ts",
        "import * as verify from 'node:assert'; test('x', () => { verify.equal(read(), 1); });",
        "",
    ),
    (
        "case.test.ts",
        "const verify = require('node:assert/strict'); test('x', () => { verify.equal(read(), 1); });",
        "",
    ),
    (
        "case.test.ts",
        "import {equal as verify} from 'unrelated'; test('x', () => { verify(read(), 1); });",
        "ts/no-assertionless-tests",
    ),
    (
        "case.test.ts",
        "test('x', () => { const x = read(); expect(x).toBe(x); });",
        "ts/no-assertionless-tests",
    ),
    (
        "case.test.ts",
        "test('x', () => { const x = read(); expect((0, x)).toBe(x); });",
        "ts/no-assertionless-tests",
    ),
    (
        "case.test.ts",
        "test('x', () => { const x = read(); const alias = x; expect(x).toBe(alias); });",
        "ts/no-assertionless-tests",
    ),
    (
        "case.test.ts",
        "const value = read(); const matcher = expect(value).toBe; test('x', () => { const value = other(); matcher(value); });",
        "",
    ),
    (
        "case.test.ts",
        "vi.waitFor(() => expect(calls).toHaveLength(1));",
        "ts/no-unsignaled-poll",
    ),
    (
        "case.test.ts",
        "vi.waitFor(() => (0, expect(calls)).toHaveLength(1));",
        "ts/no-unsignaled-poll",
    ),
    (
        "case.test.ts",
        "const check = expect.bind(undefined); vi.waitFor(() => check(calls).toHaveLength(1));",
        "ts/no-unsignaled-poll",
    ),
    (
        "case.test.ts",
        "vi.waitFor(() => expect(calls).toHaveLength.call(undefined, 1));",
        "ts/no-unsignaled-poll",
    ),
    (
        "case.test.ts",
        "const size = calls.length; expect.poll(() => size).toBe(1);",
        "ts/no-unsignaled-poll",
    ),
    (
        "case.test.ts",
        "const size = calls.current; expect.poll(() => size).toBe(1);",
        "",
    ),
    (
        "case.test.ts",
        "test('x', () => { pending(); assert(read()); });",
        "ts/no-skipped-tests",
    ),
    (
        "case.test.ts",
        "test('x', () => { const pending = save; pending(); assert(read()); });",
        "",
    ),
    (
        "case.test.ts",
        "function pending() {} test('x', () => { pending(); assert(read()); });",
        "",
    ),
    (
        "case.test.ts",
        "test('x', async () => { await new Promise(r => setTimeout(() => r(), 10)); assert(read()); });",
        "ts/no-fixed-sleep",
    ),
    (
        "case.test.ts",
        "test('x', async () => { await new Promise(r => setTimeout(() => { r(); }, 10)); assert(read()); });",
        "ts/no-fixed-sleep",
    ),
    (
        "case.test.ts",
        "test('x', async () => { await new Promise(r => setTimeout(function () { r(); }, 10)); assert(read()); });",
        "ts/no-fixed-sleep",
    ),
    (
        "runtime.ts",
        "void typeof expect;",
        "ts/no-runtime-typeof ts/no-test-aware-code",
    ),
    (
        "runtime.ts",
        "void typeof (0, expect);",
        "ts/no-runtime-typeof ts/no-test-aware-code",
    ),
    (
        "runtime.ts",
        "const check = expect; void typeof check;",
        "ts/no-runtime-typeof ts/no-test-aware-code",
    ),
    (
        "vitest.config.ts",
        "export default {outboundService: () => new Response('ok')};",
        "ts/no-test-doubles",
    ),
    (
        "vitest.config.ts",
        "export default {outboundService: () => new globalThis.Response('ok')};",
        "ts/no-test-doubles",
    ),
    (
        "vitest.config.ts",
        "const {Response: Reply} = globalThis; export default {outboundService: () => new Reply('ok')};",
        "ts/no-test-doubles",
    ),
    (
        "case.test.ts",
        "import {test} from 'node:test'; test('x', {skip: 'feature missing'}, () => assert(read()));",
        "ts/no-skipped-tests",
    ),
    (
        "case.test.ts",
        "import {test} from 'node:test'; test({skip: true}, () => assert(read()));",
        "ts/no-skipped-tests",
    ),
    (
        "case.test.ts",
        "import {test} from 'node:test'; test('x', {todo: true}, () => {});",
        "ts/no-skipped-tests",
    ),
    (
        "case.test.ts",
        "import {test} from 'node:test'; test('x', {skip: ''}, () => assert(read()));",
        "",
    ),
    (
        "case.test.ts",
        "import {test} from 'node:test'; test('x', {[key]: true}, () => assert(read()));",
        "ts/no-flake-masking ts/no-skipped-tests",
    ),
];

#[test]
fn runner_parity() {
    let dir = repo();
    let mut mismatches = Vec::new();
    for &(file, source, rule) in RUNNER_CASES {
        fs::write(dir.path().join(file), source).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(dir.path())
            .args(["check", file])
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let status = i32::from(!rule.is_empty());
        let actual: Vec<_> = stdout
            .lines()
            .filter(|line| line.starts_with(file))
            .filter_map(|line| line.split_whitespace().nth(1))
            .collect();
        let matches = actual.into_iter().eq(rule.split_whitespace());
        if !matches || out.status.code() != Some(status) || !out.stderr.is_empty() {
            mismatches.push(format!(
                "{file}: {source}\nexpected [{rule}], got {}: {stdout}{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn reflected_runner_findings_point_at_the_target() {
    let dir = repo();
    let file = "position.test.ts";
    fs::write(
        dir.path().join(file),
        concat!(
            "import * as t from 'vitest';\n",
            "Reflect.apply(t.test, null, ['x', () => {}]);\n",
            "Reflect.apply(test.slow, test, []);\n",
            "Reflect.apply(vi.fn, vi, [() => 1]);\n",
        ),
    )
    .unwrap();
    let out = check(dir.path(), &[file], 1);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with(file))
            .collect::<Vec<_>>(),
        [
            "position.test.ts:2:1 ts/no-reflect-apply",
            "position.test.ts:2:17 ts/no-assertionless-tests",
            "position.test.ts:3:1 ts/no-reflect-apply",
            "position.test.ts:3:20 ts/no-flake-masking",
            "position.test.ts:4:1 ts/no-reflect-apply",
            "position.test.ts:4:18 ts/no-test-doubles",
        ]
    );
}

#[test]
fn linked_worktree_locks_and_updates_its_own_ledger() {
    let main = repo();
    fs::write(main.path().join("a.py"), "").unwrap();
    commit(main.path());
    let outside = tempdir().unwrap();
    let linked = outside.path().join("linked");
    git(
        main.path(),
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let kept = baseline(main.path(), &[("gone.py", 1)]);
    let main_lock = fs::File::open(main.path()).unwrap();
    main_lock.lock().unwrap();
    let linked_lock = fs::File::open(&linked).unwrap();
    linked_lock.lock().unwrap();
    let before = baseline(&linked, &[("gone.py", 1)]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(&linked)
        .arg("ledger")
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(500));
    assert!(child.try_wait().unwrap().is_none());
    assert_eq!(fs::read(linked.join(".blackhole/ledger")).unwrap(), before);
    linked_lock.unlock().unwrap();
    wait_for_exit(&mut child);
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read(linked.join(".blackhole/ledger")).unwrap(),
        HEADER.as_bytes()
    );
    assert_eq!(
        fs::read(main.path().join(".blackhole/ledger")).unwrap(),
        kept
    );
}

fn wait_for_exit(child: &mut std::process::Child) {
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("command did not exit while the lock was held");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn mismatched_root_is_rejected_before_locking() {
    let main = repo();
    let nested = main.path().join("nested");
    git(main.path(), &["init", "-q", "nested"]);
    git(
        &nested,
        &["config", "core.worktree", main.path().to_str().unwrap()],
    );
    let before = baseline(main.path(), &[("gone.py", 1)]);
    let lock = fs::File::open(main.path()).unwrap();
    lock.lock().unwrap();
    for args in [
        &["ledger"][..],
        &["check"],
        &["object", RULE, "gone.py", "reason"],
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(&nested)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        wait_for_exit(&mut child);
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert_eq!(out.stdout, b"");
        assert_eq!(out.stderr, MISMATCH);
        assert_eq!(
            fs::read(main.path().join(".blackhole/ledger")).unwrap(),
            before
        );
        assert_eq!(names(&nested), [".git"]);
    }
}

#[test]
fn configured_worktree_must_resolve_to_the_checkout() {
    let dir = repo();
    let other = repo();
    let root = dir.path();
    let sub = root.join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(root.join("a.py"), "#noqa").unwrap();
    let expected = check(root, &[], 1).stdout;
    assert_eq!(check(&sub, &[], 1).stdout, expected);
    // Git walks past an entry named `.git` that is no repository; the root it then answers is not the nearest.
    let stray = root.join("stray");
    fs::create_dir_all(stray.join(".git")).unwrap();
    assert_eq!(check(&stray, &[], 2).stderr, MISMATCH);
    // A relative value is read from the git directory, so this one names the checkout itself.
    git(root, &["config", "core.worktree", ".."]);
    assert_eq!(check(&sub, &[], 1).stdout, expected);
    for value in [other.path().to_str().unwrap(), sub.to_str().unwrap(), "."] {
        git(root, &["config", "core.worktree", value]);
        assert_eq!(check(root, &[], 2).stderr, MISMATCH);
        assert_eq!(names(other.path()), [".git"]);
    }
}

// A program the checkout's configuration names, which leaves a mark outside the checkout when it runs.
#[cfg(unix)]
fn program(root: &Path, marker: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    // Git hands the configured value to a shell, as the script does the marker.
    let quoted = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', r"'\''"));
    let path = root.join(".git/program.sh");
    fs::write(&path, format!("#!/bin/sh\ntouch {}\n", quoted(marker))).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    quoted(&path)
}

#[cfg(unix)]
#[test]
fn a_configured_monitor_does_not_run() {
    let outside = tempdir().unwrap();
    let marker = outside.path().join("ran");
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "#noqa").unwrap();
    git(root, &["add", "a.py"]);
    git(root, &["config", "core.fsmonitor", &program(root, &marker)]);
    // Plain git runs the program here, so a fixture that names it wrongly fails instead of passing.
    git(root, &["ls-files", "-co", "--exclude-standard"]);
    fs::remove_file(&marker).unwrap();
    check(root, &[], 1);
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn a_missing_object_is_not_fetched() {
    let outside = tempdir().unwrap();
    let marker = outside.path().join("ran");
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "x = 1\n").unwrap();
    commit(root);
    // Git asks a promisor remote for an object it lacks, with the program the configuration names.
    for (key, value) in [
        ("extensions.partialClone", "origin"),
        ("remote.origin.promisor", "true"),
        ("remote.origin.url", root.to_str().unwrap()),
        ("remote.origin.uploadpack", &program(root, &marker)),
    ] {
        git(root, &["config", key, value]);
    }
    // An id of the repository's own length, or git takes it for an abbreviation and never looks the object up.
    let head = git_output(root, &["rev-parse", "HEAD"]).stdout;
    let missing = "1".repeat(head.trim_ascii_end().len());
    // Plain git runs the program here, so a fixture that names it wrongly fails instead of passing.
    let fetched = git_output(
        root,
        &["rev-parse", "--verify", &format!("{missing}^{{commit}}")],
    );
    assert!(!fetched.status.success());
    fs::remove_file(&marker).unwrap();
    run(root, &["ci", "--base", &missing, "--head", "HEAD"], 2);
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn a_directory_link_under_tests_is_not_a_removed_test_file() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join("tests")).unwrap();
    fs::write(root.join("tests/data.txt"), "sample").unwrap();
    std::os::unix::fs::symlink(".", root.join("tests/assets")).unwrap();
    commit(root);
    fs::remove_file(root.join("tests/assets")).unwrap();
    commit(root);
    run(root, &["ci", "--base", "HEAD~", "--head", "HEAD"], 0);
}

#[cfg(unix)]
#[test]
fn ci_accepts_a_repaired_test_symlink() {
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink("cases.ts", root.join("cases.test.ts")).unwrap();
    commit(root);
    fs::write(
        root.join("cases.ts"),
        "test(\"first\", () => expect(first()).toBe(true));\n",
    )
    .unwrap();
    commit(root);
    let out = run(root, &["ci", "--base", "HEAD~", "--head", "HEAD"], 0);
    assert_eq!(out.stdout, b"");
}
