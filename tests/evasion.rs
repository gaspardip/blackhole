use std::{fs, io, path::Path, process::Command};
use tempfile::tempdir;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
        .output()?;
    if !out.status.success() {
        return Err(String::from_utf8(out.stderr)?.into());
    }
    Ok(String::from_utf8(out.stdout)?.trim().into())
}

fn copy(src: &Path, dest: &Path) -> io::Result<()> {
    for entry in fs::read_dir(src)? {
        let e = entry?;
        let p = dest.join(e.file_name());
        if e.file_type()?.is_dir() {
            fs::create_dir(&p)?;
            copy(&e.path(), &p)?;
        } else if e.file_name() != ".keep" {
            fs::copy(e.path(), p)?;
        }
    }
    Ok(())
}

fn commit(case: &Path, root: &Path, side: &str) -> Result<String> {
    copy(&case.join(side), root)?;
    git(root, &["add", "-A"])?;
    git(root, &["commit", "--allow-empty", "-qm", side])?;
    git(root, &["rev-parse", "HEAD"])
}

fn run(case: &Path) -> Result<()> {
    let temp = tempdir()?;
    let root = temp.path();
    git(root, &["init", "-q"])?;
    let base = commit(case, root, "base")?;
    git(root, &["rm", "-rq", "--ignore-unmatch", "."])?;
    let head = commit(case, root, "head")?;
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
    if out.status.code() != Some(code)
        || stdout.lines().last().unwrap_or("") != summary
        || !out.stderr.is_empty()
    {
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
