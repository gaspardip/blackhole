use crate::common::{command, git};
use std::{
    io::{self, Write},
    path::Path,
    process::{Command, Output, Stdio},
};

/// # Panics
/// Panics when the process cannot run or returns an unexpected status.
#[must_use]
pub fn run(root: &Path, args: &[&str], data: &str, status: i32, partial: bool) -> Output {
    run_input(
        command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .args(args),
        data.as_bytes(),
        status,
        partial,
    )
}

/// # Panics
/// Panics on incomplete delivery or an unexpected child status.
#[must_use]
pub fn run_input(cmd: &mut Command, data: &[u8], status: i32, partial: bool) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Err(error) = child.stdin.take().unwrap().write_all(data) {
        assert!(
            partial && error.kind() == io::ErrorKind::BrokenPipe,
            "incomplete test input: {error}"
        );
    }
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(status), "{out:?}");
    out
}

/// # Panics
/// Panics if the temporary repository cannot be initialized.
#[must_use]
pub fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]).unwrap();
    dir
}
