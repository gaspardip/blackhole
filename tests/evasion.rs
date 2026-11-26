mod common;
mod trees;
use common::{Result, git};
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

fn run(case: &Path) -> Result<()> {
    let temp = tempdir()?;
    let root = temp.path();
    let (base, head) = trees::revisions(case, root)?;
    let expected = fs::read_to_string(case.join("expect"))?;
    let mut lines = expected.lines();
    let gates = lines.next().ok_or("missing verdict")?;
    let day = lines
        .next()
        .map(|s| s.strip_prefix("today ").ok_or("invalid today"))
        .transpose()?
        .unwrap_or("2026-11-18");
    if lines.next().is_some() {
        return Err("unexpected expect line".into());
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_blackhole"));
    cmd.current_dir(root)
        .args(["ci", "--base", &base, "--head", &head, "--today", day]);
    let artifacts = tempdir()?;
    let report = case.join("lcov.info");
    if report.try_exists()? {
        let dir = artifacts.path().join(&head).join("reports");
        fs::create_dir_all(&dir)?;
        fs::copy(report, dir.join("lcov.info"))?;
        cmd.arg("--coverage-dir").arg(artifacts.path());
    }
    let approvals = case.join("approvals.json");
    if approvals.try_exists()? {
        let mut data: serde_json::Value = serde_json::from_slice(&fs::read(approvals)?)?;
        for r in data["reviews"].as_array_mut().ok_or("invalid reviews")? {
            match r["commit"].as_str() {
                Some("BASE") => r["commit"] = base.clone().into(),
                Some("HEAD") => r["commit"] = head.clone().into(),
                _ => {}
            }
        }
        let p = root.join("approvals.json");
        fs::write(&p, serde_json::to_vec(&data)?)?;
        cmd.arg("--approvals").arg(p);
    }
    let (code, summary) = if gates == "green" {
        (0, String::new())
    } else {
        (
            1,
            format!(
                "blackhole ci: red ({})",
                gates
                    .strip_prefix("red ")
                    .ok_or("invalid verdict")?
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    };
    let out = cmd.output()?;
    let stdout = String::from_utf8(out.stdout)?;
    let matches_verdict = if gates == "green" {
        let advisory = case.join("advisory");
        let expected_stdout = if advisory.try_exists()? {
            fs::read_to_string(advisory)?
        } else {
            String::new()
        };
        stdout == expected_stdout
    } else {
        stdout.lines().last().unwrap_or("") == summary
    };
    if out.status.code() != Some(code) || !matches_verdict || !out.stderr.is_empty() {
        return Err(format!(
            "expected {gates}, got {} {stdout}{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    Ok(())
}

#[test]
fn evasion() -> Result<()> {
    let mut failures = Vec::new();
    let mut count = 0;
    for entry in fs::read_dir("evasion")? {
        let path = entry?.path();
        count += 1;
        if let Err(e) = run(&path) {
            failures.push(format!("{}: {e}", path.display()));
        }
    }
    assert!(count > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

fn coverage_verdict(
    repo: &Path,
    base: &str,
    head: &str,
    artifacts: &Path,
    expected: &str,
) -> Result<()> {
    let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(repo)
        .args(["ci", "--base", base, "--head", head, "--coverage-dir"])
        .arg(artifacts)
        .output()?;
    let expected_code = i32::from(!expected.is_empty());
    assert_eq!(out.status.code(), Some(expected_code), "{out:?}");
    if expected_code == 1 {
        let stdout = String::from_utf8(out.stdout)?;
        assert!(
            stdout.contains(expected) && stdout.ends_with("blackhole ci: red (coverage)\n"),
            "{stdout}"
        );
    } else {
        assert_eq!(out.stdout, b"");
    }
    assert_eq!(out.stderr, b"");
    Ok(())
}

#[test]
fn coverage_follows_repository_paths_in_subdirectories_and_bare_clones() -> Result<()> {
    let repo = tempdir()?;
    let root = repo.path();
    let case = Path::new("evasion/coverage-changed");
    let (base, head) = trees::revisions(case, root)?;
    let artifacts = tempdir()?;
    let dir = artifacts.path().join(&head).join("reports");
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join("lcov.info"),
        "SF:src/value.ts\nDA:2,0\nend_of_record\n",
    )?;
    let mirror = tempdir()?;
    git(
        root,
        &["clone", "--bare", ".", mirror.path().to_str().unwrap()],
    )?;
    git(root, &["config", "diff.relative", "true"])?;
    for cwd in [root.join("src"), mirror.path().to_owned()] {
        coverage_verdict(
            &cwd,
            &base,
            &head,
            artifacts.path(),
            "src/value.ts:2: changed line",
        )?;
    }
    fs::copy(case.join("lcov.info"), dir.join("lcov.info"))?;
    coverage_verdict(mirror.path(), &base, &head, artifacts.path(), "")?;
    Ok(())
}

#[test]
fn coverage_requires_complete_line_records_for_the_changed_source() -> Result<()> {
    let repo = tempdir()?;
    let root = repo.path();
    let (base, head) = trees::revisions(Path::new("evasion/coverage-changed"), root)?;
    let artifacts = tempdir()?;
    let dir = artifacts.path().join(&head).join("reports");
    fs::create_dir_all(&dir)?;
    for (record, expected) in [
        ("", "invalid"),
        ("DA:2,1\nend_of_record", "invalid"),
        ("SF:src/value.ts\nDA:2,1", "invalid"),
        ("SF:src/value.ts\nDA:0,1\nend_of_record", "invalid"),
        ("SF:src/value.ts\nDA:2,-1\nend_of_record", "invalid"),
        ("SF:src/value.ts\nDA:2,1\nDA:2,0\nend_of_record", "invalid"),
        ("SF:src/value.ts\nLF:1\nLH:1\nend_of_record", "invalid"),
        ("SF:src/value.ts\nDA:2,1\nLF:2\nend_of_record", "invalid"),
        ("SF:src/value.ts\nDA:2,1\nLH:0\nend_of_record", "invalid"),
        ("SF:src/value.ts\nend_of_record", "invalid"),
        (
            "SF:src/other.ts\nDA:2,1\nend_of_record",
            "missing LCOV record",
        ),
        ("SF:src/value.ts\nDA:4,1\nend_of_record", "beyond end"),
        (
            "SF:src/value.ts\nDA:1,1\nDA:2,0\nend_of_record",
            "src/value.ts:2: changed line",
        ),
        (
            "SF:./src/value.ts\r\nDA:2,0\r\nend_of_record\r\nSF:src/value.ts\nDA:2,3,checksum\nend_of_record",
            "",
        ),
    ] {
        fs::write(dir.join("lcov.info"), record)?;
        coverage_verdict(root, &base, &head, artifacts.path(), expected)?;
    }
    Ok(())
}

#[test]
fn coverage_reads_only_the_external_artifact_for_the_exact_head() -> Result<()> {
    let temp = tempdir()?;
    let root = temp.path().join("repo");
    fs::create_dir(&root)?;
    let case = Path::new("evasion/coverage-changed");
    let (base, head) = trees::revisions(case, &root)?;
    let stale = temp.path().join(&base).join("reports");
    fs::create_dir_all(&stale)?;
    fs::copy(case.join("lcov.info"), stale.join("lcov.info"))?;
    coverage_verdict(&root, &base, &head, temp.path(), "missing")?;

    // A runner can name its checkout after the commit.
    let checkout = temp.path().join(&head);
    fs::rename(&root, &checkout)?;
    fs::create_dir(checkout.join("reports"))?;
    fs::copy(case.join("lcov.info"), checkout.join("reports/lcov.info"))?;
    coverage_verdict(&checkout, &base, &head, temp.path(), "outside the checkout")?;
    #[cfg(unix)]
    {
        let external = tempdir()?;
        std::os::unix::fs::symlink(&checkout, external.path().join(&head))?;
        coverage_verdict(
            &checkout,
            &base,
            &head,
            external.path(),
            "outside the checkout",
        )?;
    }
    Ok(())
}
