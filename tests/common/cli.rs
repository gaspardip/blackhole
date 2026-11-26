use super::common::{command, git};
use std::{path::Path, process::Output};
use tempfile::{TempDir, tempdir};

pub fn repo() -> TempDir {
    let dir = tempdir().unwrap();
    git(dir.path(), &["init", "-q", "--template="]).unwrap();
    dir
}

pub fn check(dir: &Path, args: &[&str], code: i32) -> Output {
    run(dir, &[&["check"], args].concat(), code)
}

pub fn run(dir: &Path, args: &[&str], code: i32) -> Output {
    let out = command(env!("CARGO_BIN_EXE_blackhole"))
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
