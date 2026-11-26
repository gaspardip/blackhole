mod common;

use blackhole_check::engine::registry;
use common::{Result, command, git};
use std::{fs, path::Path};
use tempfile::tempdir;

fn track(root: &Path) -> Result<()> {
    git(root, &["init", "-q", "--template="])?;
    // Tracked fixtures must stay visible even when Git would ignore them.
    git(root, &["add", "--force", "--", "."])?;
    Ok(())
}

fn populate(source: &Path, root: &Path, tree: &str) -> Result<Vec<String>> {
    let tracked = git(source, &["ls-files", "--deduplicate", "-z", "--", tree])?;
    let mut configured = Vec::new();
    for path in tracked.split_terminator('\0') {
        let mut components = path.split('/');
        if tree == "evasion" && !matches!(components.nth(2), Some("base" | "head")) {
            continue;
        }
        if tree == "rules"
            && let Some(dir) = path.strip_suffix("/blackhole.toml")
            && dir.ends_with("/tree")
            && let ["rules", _, _, "accept" | "reject", _, "tree"] =
                dir.split('/').collect::<Vec<_>>().as_slice()
        {
            configured.push(dir.to_owned());
        }
        let target = root.join(path);
        fs::create_dir_all(target.parent().ok_or("missing fixture directory")?)?;
        fs::copy(source.join(path), target)?;
    }
    track(root)?;
    Ok(configured)
}

fn findings(root: &Path, tree: &str) -> Result<Vec<String>> {
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
    let stdout = String::from_utf8(out.stdout)?;
    Ok(stdout
        .lines()
        .filter(|line| {
            !line
                .split_once(' ')
                .is_some_and(|(id, _)| registry().iter().any(|rule| rule.id == id))
        })
        .map(str::to_owned)
        .collect())
}

fn verify(tree: &str) -> Result<()> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = tempdir()?;
    let configured = populate(source, repo.path(), tree)?;
    let mut lines = findings(repo.path(), tree)?;
    for dir in configured {
        let root = repo.path().join(&dir);
        track(&root)?;
        let prefix = format!("{dir}/");
        lines.retain(|line| !line.starts_with(&prefix));
        lines.extend(
            findings(&root, &dir)?
                .into_iter()
                .map(|line| format!("{prefix}{line}")),
        );
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
