mod common;
use common::{Result, commit, git};
use std::{env, fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use tempfile::tempdir;

const TOKEN: &str = "not-a-real-token";

struct Run {
    green: bool,
    said: String,
    calls: String,
}

fn stub(bin: &Path, name: &str, body: &str) -> Result<()> {
    let path = bin.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn judge(case: &str, target: &str) -> Result<Run> {
    let temp = tempdir()?;
    let root = temp.path();
    let case = Path::new("evasion").join(case);
    let (origin, work, bin) = (root.join("o/r.git"), root.join("work"), root.join("bin"));
    for dir in [&origin, &work, &bin] {
        fs::create_dir_all(dir)?;
    }
    let server = format!("file://{}", root.display());
    let url = format!("{server}/o/r.git");
    git(&origin, &["init", "-q", "--bare", "--initial-branch=main"])?;
    git(&work, &["init", "-q"])?;
    commit(&case, &work, "base")?;
    git(&work, &["push", "-q", &url, "HEAD:refs/heads/main"])?;
    git(&work, &["rm", "-rq", "--ignore-unmatch", "."])?;
    let head = commit(&case, &work, "head")?;
    git(&work, &["push", "-q", &url, "HEAD:refs/pull/1/head"])?;
    let calls = root.join("calls");
    fs::write(&calls, "")?;
    // A real build takes minutes and proves nothing about the script, so the judge is the binary under test.
    stub(
        &bin,
        "cargo",
        r#"[ -z "${GH_TOKEN+x}" ] || echo "cargo sees the token" >> "$CALLS"
mkdir -p target/release && ln -s "$BLACKHOLE" target/release/blackhole"#,
    )?;
    stub(&bin, "gh", r#"echo "$*" >> "$CALLS""#)?;
    let out = Command::new("bash")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/judge.sh"))
        .current_dir(root)
        .env("PATH", format!("{}:{}", bin.display(), env::var("PATH")?))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("CALLS", &calls)
        .env("BLACKHOLE", env!("CARGO_BIN_EXE_blackhole"))
        .env("GH_TOKEN", TOKEN)
        .env("BASE_REF", target)
        .env("HEAD_SHA", &head)
        .env("NUMBER", "1")
        .env("GITHUB_REPOSITORY", "o/r")
        .env("GITHUB_SERVER_URL", &server)
        .env("GITHUB_RUN_ID", "7")
        .output()?;
    Ok(Run {
        green: out.status.success(),
        said: String::from_utf8(out.stdout)? + &String::from_utf8(out.stderr)?,
        calls: fs::read_to_string(calls)?
            .replace(&head, "HEAD")
            .replace(&server, "SERVER"),
    })
}

#[test]
fn the_verdict_is_one_status_named_for_the_target() -> Result<()> {
    for (case, state) in [("clean", "success"), ("edit-workflow", "failure")] {
        let run = judge(case, "main")?;
        assert_eq!(run.green, state == "success", "{case}: {}", run.said);
        assert_eq!(
            run.calls,
            format!(
                "api -X POST repos/o/r/statuses/HEAD -f context=judge (main) -f state={state} -f description=blackhole ci -f target_url=SERVER/o/r/actions/runs/7\n"
            ),
            "{case}"
        );
        assert!(!run.said.contains(TOKEN), "{case}: {}", run.said);
    }
    Ok(())
}

#[test]
fn a_target_another_branch_could_share_a_status_name_with_is_refused() -> Result<()> {
    for target in ["Main", "ma\u{ed}n", "release^1", "a b"] {
        let run = judge("clean", target)?;
        assert!(
            !run.green && run.said.contains("cannot be judged") && run.calls.is_empty(),
            "{target}: {}{}",
            run.said,
            run.calls
        );
    }
    Ok(())
}
