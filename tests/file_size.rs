mod common;

use common::{Result, command, git};
use std::{fs, path::Path};
use tempfile::{TempDir, tempdir};

fn repo() -> Result<TempDir> {
    let dir = tempdir()?;
    git(dir.path(), &["init", "-q", "--template="])?;
    Ok(dir)
}

fn check(root: &Path, paths: &[&str], code: i32) -> Result<String> {
    let out = command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(root)
        .arg("check")
        .args(paths)
        .output()?;
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    Ok(String::from_utf8(out.stdout)?)
}

#[test]
fn lines_are_counted_on_the_text_as_decoded() -> Result<()> {
    let dir = repo()?;
    let root = dir.path();
    fs::write(root.join("blackhole.toml"), "[boundaries]\nmax-lines = 3")?;
    for (bytes, expected) in [
        (&b"a\r\0\nb\rc"[..], true),
        (&b"a\r\nb\rc"[..], false),
        (&b"\0a\n\0\n\0\nb\0"[..], true),
        (&b"a\nb\nc\n\0"[..], true),
        ("a\u{2028}b\u{2029}c\nd\r\n".as_bytes(), true),
        ("a\u{2028}b\u{2029}c\u{2028}".as_bytes(), false),
        (&b"a\nb\nc\n\xff"[..], false),
    ] {
        fs::write(root.join("x.txt"), bytes)?;
        let out = check(root, &["x.txt"], i32::from(expected))?;
        if expected {
            assert_eq!(
                out.lines().next(),
                Some("x.txt:4:1 boundary/file-size 4 lines, limit 3")
            );
        } else {
            assert_eq!(out, "");
        }
    }
    Ok(())
}

#[test]
fn the_limit_is_opt_in_and_applies_to_selected_generated_text() -> Result<()> {
    let dir = repo()?;
    let root = dir.path();
    for name in ["generated.txt", "excluded.txt"] {
        fs::write(root.join(name), "generated\na\nb\nc\nd\n")?;
    }
    assert_eq!(check(root, &[], 0)?, "");
    fs::write(
        root.join("blackhole.toml"),
        "exclude = ['excluded.txt']\n[boundaries]\nmax-lines = 3",
    )?;
    let out = check(root, &[], 1)?;
    let findings: Vec<_> = out.lines().filter(|line| line.contains(".txt:")).collect();
    assert_eq!(
        findings,
        ["generated.txt:4:1 boundary/file-size 5 lines, limit 3"]
    );
    assert_eq!(check(root, &["excluded.txt"], 0)?, "");
    Ok(())
}

#[test]
fn invalid_limits_are_configuration_errors() -> Result<()> {
    let dir = repo()?;
    let root = dir.path();
    for value in ["0", "-1", "'3'"] {
        fs::write(
            root.join("blackhole.toml"),
            format!("[boundaries]\nmax-lines = {value}"),
        )?;
        let out = command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .arg("check")
            .output()?;
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        let error = String::from_utf8(out.stderr)?;
        assert!(error.contains("nonzero usize"), "{error}");
    }
    Ok(())
}
