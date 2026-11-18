use std::{
    fs,
    path::Path,
    process::{Command, Output},
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
    let out = Command::new(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(dir)
        .arg("check")
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
