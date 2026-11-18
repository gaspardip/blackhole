use std::{
    fs,
    path::Path,
    process::{Command, Output},
    thread,
    time::Duration,
};
use tempfile::{TempDir, tempdir};

fn repo() -> TempDir {
    let dir = tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn git(dir: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}

fn check(dir: &Path, args: &[&str], code: i32) -> Output {
    run(dir, &[&["check"], args].concat(), code)
}

fn run(dir: &Path, args: &[&str], code: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    if code == 2 {
        assert_eq!(String::from_utf8_lossy(&out.stderr).lines().count(), 1);
    } else {
        assert_eq!(out.stderr, b"");
    }
    out
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
    for (name, value) in [
        ("GIT_DIR", other.path().join(".git")),
        ("GIT_WORK_TREE", other.path().to_owned()),
        ("GIT_INDEX_FILE", other.path().join("index")),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .env(name, value)
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
    // A caller's git variables must not move the lock away from the ledger it guards.
    for separate in [false, true] {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("a.py"), "#noqa").unwrap();
        let before = baseline(root, &[("a.py", 2)]);
        let lock = fs::File::create(root.join(".git/blackhole.lock")).unwrap();
        lock.lock().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_blackhole"));
        command.current_dir(root).arg("ledger");
        if separate {
            command
                .env("GIT_DIR", other.path().join(".git"))
                .env("GIT_WORK_TREE", root);
        }
        let mut child = command.spawn().unwrap();
        thread::sleep(Duration::from_millis(500));
        assert!(child.try_wait().unwrap().is_none(), "{separate}");
        assert_eq!(fs::read(root.join(".blackhole/ledger")).unwrap(), before);
        lock.unlock().unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            fs::read(root.join(".blackhole/ledger")).unwrap(),
            format!("{HEADER}{RULE}\t1\ta.py\n").as_bytes()
        );
    }
}

#[cfg(unix)]
#[test]
fn a_linked_lock_is_refused() {
    let dir = repo();
    let root = dir.path();
    let other = tempdir().unwrap();
    let victim = other.path().join("victim");
    fs::write(&victim, "kept").unwrap();
    fs::write(root.join("a.py"), "#noqa").unwrap();
    let before = baseline(root, &[("a.py", 2)]);
    std::os::unix::fs::symlink(&victim, root.join(".git/blackhole.lock")).unwrap();
    run(root, &["ledger"], 2);
    assert_eq!(fs::read(&victim).unwrap(), b"kept");
    assert_eq!(fs::read(root.join(".blackhole/ledger")).unwrap(), before);
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
