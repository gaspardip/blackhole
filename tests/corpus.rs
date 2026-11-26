mod common;

use common::{Result, command, git};
use std::{fs, path::Path};
use tempfile::tempdir;

fn populate(source: &Path, root: &Path, tree: &str, strip: &Path) -> Result<()> {
    git(root, &["init", "-q", "--template="])?;
    let tracked = git(source, &["ls-files", "--deduplicate", "-z", "--", tree])?;
    for path in tracked.split_terminator('\0') {
        let mut components = path.split('/');
        if tree == "evasion" && !matches!(components.nth(2), Some("base" | "head")) {
            continue;
        }
        let target = root.join(Path::new(path).strip_prefix(strip)?);
        fs::create_dir_all(target.parent().ok_or("missing fixture directory")?)?;
        fs::copy(source.join(path), target)?;
    }
    // Tracked fixtures must stay visible even when Git would ignore them.
    git(root, &["add", "--force", "--", "."])?;
    Ok(())
}

fn findings(root: &Path, tree: &str) -> Result<String> {
    let out = command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(root)
        .arg("check")
        .output()?;
    assert!(
        matches!(out.status.code(), Some(0 | 1)) && out.stderr.is_empty(),
        "blackhole check on {tree} exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}

fn boundary_fixture_findings(source: &Path, rule: &str) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let reject = source.join(format!("rules/{rule}/reject"));
    let marker = format!(" {rule} ");
    for case in fs::read_dir(reject)? {
        let tree = case?.path().join("tree");
        let path = tree.strip_prefix(source)?;
        let prefix = path.to_str().ok_or("fixture path is not UTF-8")?;
        let repo = tempdir()?;
        populate(source, repo.path(), prefix, path)?;
        let out = findings(repo.path(), prefix)?;
        lines.extend(
            out.lines()
                .filter(|line| line.contains(&marker))
                .map(|line| format!("{prefix}/{line}")),
        );
    }
    Ok(lines)
}

fn verify(tree: &str) -> Result<()> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = tempdir()?;
    populate(source, repo.path(), tree, Path::new(""))?;
    let stdout = findings(repo.path(), tree)?;
    let prefix = format!("{tree}/");
    let mut lines: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with(&prefix))
        .map(str::to_owned)
        .collect();
    if tree == "rules" {
        for rule in ["boundary/banned-imports", "boundary/folder-imports"] {
            lines.extend(boundary_fixture_findings(source, rule)?);
        }
    }
    lines.sort_unstable();
    let mut actual = String::new();
    for line in lines {
        actual.push_str(&line);
        actual.push('\n');
    }
    let snapshot = source.join("tests/corpus").join(format!("{tree}.snapshot"));
    if std::env::var("BLACKHOLE_BLESS").as_deref() == Ok("1") {
        fs::write(snapshot, actual)?;
        return Ok(());
    }
    // A checkout with core.autocrlf turns the snapshot's line endings into CRLF.
    let expected = fs::read_to_string(&snapshot)?.replace("\r\n", "\n");
    if expected == actual {
        return Ok(());
    }
    fs::write(repo.path().join("expected.snapshot"), expected)?;
    fs::write(repo.path().join("actual.snapshot"), actual)?;
    let diff = command("git")
        .current_dir(repo.path())
        .args([
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-color",
            "--text",
            "--",
            "expected.snapshot",
            "actual.snapshot",
        ])
        .output()?;
    assert_eq!(
        diff.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&diff.stderr)
    );
    panic!(
        "{} changed\n{}",
        snapshot.strip_prefix(source)?.display(),
        String::from_utf8(diff.stdout)?
    );
}

#[test]
fn rules_corpus() -> Result<()> {
    verify("rules")
}

#[test]
fn evasion_corpus() -> Result<()> {
    verify("evasion")
}
