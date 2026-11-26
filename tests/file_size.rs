#[path = "common/cli.rs"]
mod cli;
mod common;

use cli::{check, repo};
use common::Result;
use std::fs;

#[test]
fn lines_are_counted_on_the_text_as_decoded() -> Result<()> {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("blackhole.toml"), "[boundaries]\nmax-lines = 3")?;
    for (bytes, expected) in [
        (&b"a\r\0\nb\rc"[..], true),
        (&b"a\r\nb\rc"[..], false),
        (&b"\0a\n\0\n\0\nb\0"[..], true),
        (&b"a\nb\nc\n\0"[..], true),
        ("a\u{2028}b\u{2029}c\nd\r\n".as_bytes(), true),
        ("a\u{2028}b\u{2029}c\u{2028}".as_bytes(), false),
        (&b"a\nb\nc\n\xff"[..], true),
    ] {
        fs::write(root.join("x.txt"), bytes)?;
        let out = String::from_utf8(check(root, &["x.txt"], i32::from(expected)).stdout)?;
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
fn the_utf32_le_bom_takes_precedence_over_utf16_le() -> Result<()> {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("blackhole.toml"), "[boundaries]\nmax-lines = 3")?;
    let mut bytes = vec![0xff, 0xfe, 0, 0];
    bytes.extend(
        "a\r\nb\r\nc\nd"
            .chars()
            .flat_map(|ch| u32::from(ch).to_le_bytes()),
    );
    fs::write(root.join("x.txt"), bytes)?;
    assert_eq!(
        String::from_utf8(check(root, &["x.txt"], 1).stdout)?
            .lines()
            .next(),
        Some("x.txt:4:1 boundary/file-size 4 lines, limit 3")
    );
    Ok(())
}

#[test]
fn the_limit_is_opt_in_and_applies_to_selected_generated_text() -> Result<()> {
    let dir = repo();
    let root = dir.path();
    for name in ["generated.txt", "excluded.txt"] {
        fs::write(root.join(name), "generated\na\nb\nc\nd\n")?;
    }
    assert_eq!(check(root, &[], 0).stdout, b"");
    fs::write(
        root.join("blackhole.toml"),
        "exclude = ['excluded.txt']\n[boundaries]\nmax-lines = 3",
    )?;
    let out = String::from_utf8(check(root, &[], 1).stdout)?;
    let findings: Vec<_> = out.lines().filter(|line| line.contains(".txt:")).collect();
    assert_eq!(
        findings,
        ["generated.txt:4:1 boundary/file-size 5 lines, limit 3"]
    );
    assert_eq!(check(root, &["excluded.txt"], 0).stdout, b"");
    Ok(())
}

#[test]
fn the_ledger_and_objections_are_exempt_from_the_line_limit() -> Result<()> {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".blackhole"))?;
    fs::write(root.join("blackhole.toml"), "[boundaries]\nmax-lines = 1")?;
    fs::write(root.join("a.txt"), "a\nb\n")?;
    fs::write(root.join("b.txt"), "a\nb\n")?;
    fs::write(
        root.join(".blackhole/ledger"),
        "blackhole-ledger 1\nboundary/file-size\t1\ta.txt\nboundary/file-size\t1\tb.txt\n",
    )?;
    fs::write(
        root.join(".blackhole/objections"),
        "boundary/file-size\ta.txt\t1\t2000-01-01\t-\treason\nboundary/file-size\tb.txt\t1\t2000-01-01\t-\treason\n",
    )?;
    assert_eq!(
        String::from_utf8(check(root, &[], 1).stdout)?
            .lines()
            .next(),
        Some("blackhole.toml:2:1 boundary/file-size 2 lines, limit 1")
    );
    assert_eq!(check(root, &[".blackhole"], 0).stdout, b"");
    Ok(())
}

#[test]
fn invalid_limits_are_configuration_errors() -> Result<()> {
    let dir = repo();
    let root = dir.path();
    for value in ["0", "-1", "'3'"] {
        fs::write(
            root.join("blackhole.toml"),
            format!("[boundaries]\nmax-lines = {value}"),
        )?;
        let error = String::from_utf8(check(root, &[], 2).stderr)?;
        assert!(error.contains("nonzero usize"), "{error}");
    }
    Ok(())
}
