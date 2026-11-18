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
    std::os::unix::fs::symlink("ignored.py", root.join("link.py")).unwrap();
    assert_eq!(check(root, &[], 0).stdout, b"");
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
    check(dir.path(), &[], 2);
}
