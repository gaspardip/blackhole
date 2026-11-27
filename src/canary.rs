use crate::{
    engine::{self, Gate},
    files::{LEDGER_PATH, OBJECTIONS_PATH},
    git::{self, Batch, Tree},
    github::{self, ACTIONS_APP, Failure, Repo, Response},
    init::workflow::{self, PATH as WORKFLOW_PATH},
    ledger,
};
use serde_json::Value;
use std::{
    env,
    io::{self, Write},
    process::{Child, ChildStdin, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const POLL: Duration = Duration::from_secs(10);
const MARK: &str = "# blackhole canary forgery\n";
const FINDING: &str = "blackhole-canary-finding.py";
// Runs in its own process group with the canary's end of the pipe as input, so it cleans up after an interrupt too.
const CLEANUP: &str = r#"branch= pr=
while read -r kind value; do
  case $kind in branch) branch=1 ;; pr) pr=$value ;; esac
done
status=0
if [ -n "$pr" ]; then
  gh api --hostname "$1" --method PATCH -f state=closed "repos/$2/pulls/$pr" >/dev/null || status=1
fi
if [ -n "$branch" ]; then
  gh api --hostname "$1" --method DELETE "repos/$2/git/refs/heads/$3" >/dev/null || status=1
fi
exit "$status""#;

enum Change {
    Write(&'static str, Vec<u8>),
    Delete(String),
}

struct Forgery {
    gate: &'static str,
    what: String,
    change: Change,
}

impl Forgery {
    // Every protected forgery also has to be named by path, since the other forgeries turn that gate red too.
    fn stopped(&self, gates: &[&str], log: &str) -> bool {
        gates.contains(&self.gate)
            && match (&self.change, self.gate) {
                (Change::Write(path, _), "protected") => {
                    log.contains(&format!("protected: {path}: "))
                }
                _ => true,
            }
    }
}

fn error(failure: Failure) -> io::Error {
    match failure {
        Failure::NoSession => io::Error::other("cannot read GitHub (install and sign in to gh)"),
        Failure::Unreachable(e) => io::Error::other(format!("cannot read GitHub ({e})")),
    }
}

fn call(
    repo: &Repo,
    method: &str,
    endpoint: &str,
    args: &[&str],
    want: u16,
) -> io::Result<Response> {
    let response = github::api(&repo.host, method, endpoint, args).map_err(error)?;
    if response.status == want {
        Ok(response)
    } else {
        Err(io::Error::other(format!(
            "GitHub answered {method} {endpoint} with HTTP {}",
            response.status
        )))
    }
}

fn json(response: &Response) -> io::Result<Value> {
    response
        .json()
        .ok_or_else(|| io::Error::other("GitHub answered with invalid JSON"))
}

// The pushes and the fetch use gh's sign-in, so the canary tests the rights of the credential an agent would hold.
fn remote_git() -> Command {
    let mut command = git::command();
    command.args([
        "-c",
        "credential.helper=",
        "-c",
        "credential.helper=!gh auth git-credential",
    ]);
    command
}

fn hash(bytes: &[u8]) -> io::Result<String> {
    let mut child = git::command()
        .args(["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("git stdin"))?
        .write_all(bytes)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(git::failure(&out));
    }
    text(out.stdout)
}

fn text(bytes: Vec<u8>) -> io::Result<String> {
    let text = String::from_utf8(bytes).map_err(io::Error::other)?;
    Ok(text.trim_end().to_owned())
}

fn commit(parent: &str, forgery: &Forgery) -> io::Result<String> {
    let index = env::temp_dir().join(format!("blackhole-canary-{}.index", std::process::id()));
    let indexed = |args: &[&str]| {
        git::output(git::command().env("GIT_INDEX_FILE", &index).args(args)).and_then(text)
    };
    let result = (|| {
        indexed(&["read-tree", parent])?;
        match &forgery.change {
            Change::Write(path, bytes) => {
                let blob = hash(bytes)?;
                indexed(&[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("100644,{blob},{path}"),
                ])?;
            }
            Change::Delete(path) => {
                indexed(&["update-index", "--force-remove", "--", path])?;
            }
        }
        let tree = indexed(&["write-tree"])?;
        let message = format!("blackhole canary: {}", forgery.what);
        git::output(
            git::command()
                .envs([
                    ("GIT_AUTHOR_NAME", "blackhole canary"),
                    ("GIT_AUTHOR_EMAIL", "canary@blackhole.invalid"),
                    ("GIT_COMMITTER_NAME", "blackhole canary"),
                    ("GIT_COMMITTER_EMAIL", "canary@blackhole.invalid"),
                ])
                .args(["commit-tree", &tree, "-p", parent, "-m", &message]),
        )
        .and_then(text)
    })();
    match std::fs::remove_file(&index) {
        Err(e) if e.kind() != io::ErrorKind::NotFound && result.is_ok() => Err(e),
        _ => result,
    }
}

fn appended(existing: Option<Vec<u8>>, fresh: &[u8]) -> Vec<u8> {
    let mut bytes = existing.unwrap_or_else(|| fresh.to_vec());
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(MARK.as_bytes());
    bytes
}

// One forgery for each gate the base policy can turn red; the workflow edit comes last because GitHub may refuse its push.
fn forgeries(tip: &str) -> io::Result<Vec<Forgery>> {
    let tree = Tree::load(tip)?;
    let mut batch = Batch::start()?;
    let mut read = |path| tree.judge_file(path, &mut batch);
    let mut counts = ledger::load(read(LEDGER_PATH)?.as_deref())?;
    if let Some(count) = counts.values_mut().next() {
        *count += 1;
    } else {
        let rule = engine::registry()
            .iter()
            .find(|rule| rule.gate == Gate::Blocking && rule.run.can_allow())
            .ok_or_else(|| io::Error::other("no rule can be allowed"))?;
        counts.insert(
            (
                rule.id.to_owned(),
                "blackhole-canary-allowance.py".to_owned(),
            ),
            1,
        );
    }
    let mut list = vec![
        Forgery {
            gate: "ledger",
            what: "raise a ledger allowance".to_owned(),
            change: Change::Write(LEDGER_PATH, ledger::write(&counts)?.into_bytes()),
        },
        Forgery {
            gate: "protected",
            what: "edit blackhole.toml".to_owned(),
            change: Change::Write("blackhole.toml", appended(read("blackhole.toml")?, b"")),
        },
        Forgery {
            gate: "objections",
            what: "file an objection with an invalid shape".to_owned(),
            change: Change::Write(OBJECTIONS_PATH, appended(read(OBJECTIONS_PATH)?, b"")),
        },
    ];
    let judge = appended(read(WORKFLOW_PATH)?, workflow::content().as_bytes());
    let config = crate::config::parse(&read("blackhole.toml")?.unwrap_or_default())?;
    batch.finish()?;
    if let Some(test) = tree.paths().find(|path| {
        crate::packs::ts::test_context::test_file_path(path) && !config.exclude.is_match(path)
    }) {
        list.push(Forgery {
            gate: "tests",
            what: format!("delete {test}"),
            change: Change::Delete(test.to_owned()),
        });
    }
    list.push(Forgery {
        gate: "findings",
        what: format!("add a finding in {FINDING}"),
        change: Change::Write(FINDING, b"# noqa\n".to_vec()),
    });
    list.push(Forgery {
        gate: "protected",
        what: format!("edit {WORKFLOW_PATH}"),
        change: Change::Write(WORKFLOW_PATH, judge),
    });
    Ok(list)
}

enum Push {
    Pushed,
    Refused,
}

fn push(url: &str, sha: &str, branch: &str) -> io::Result<Push> {
    let out = remote_git()
        .args(["push", "-q", url, &format!("{sha}:refs/heads/{branch}")])
        .stdin(Stdio::null())
        .output()?;
    if out.status.success() {
        return Ok(Push::Pushed);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    // GitHub refuses a workflow change from a token without the workflow scope or permission.
    if stderr.contains("refusing to allow") && stderr.contains("workflow") {
        return Ok(Push::Refused);
    }
    Err(io::Error::other(format!(
        "git push failed: {}",
        stderr.trim_end()
    )))
}

struct Run<'a> {
    repo: &'a Repo,
    branch: &'a str,
    base: &'a str,
    timeout: Duration,
    cleanup: &'a mut ChildStdin,
}

enum Outcome {
    Judged { conclusion: String, log: String },
    TimedOut(String),
}

impl Run<'_> {
    fn note(&mut self, line: &str) -> io::Result<()> {
        writeln!(self.cleanup, "{line}")
    }

    fn judge(&mut self, head: &str, count: usize) -> io::Result<Outcome> {
        let slug = &self.repo.slug;
        let title = format!("title=blackhole canary: {count} forgeries");
        let created = call(
            self.repo,
            "POST",
            &format!("repos/{slug}/pulls"),
            &[
                "-f",
                &title,
                "-f",
                &format!("head={}", self.branch),
                "-f",
                &format!("base={}", self.base),
                "-f",
                "body=Opened by blackhole canary. It closes this pull request and deletes its branch when the blackhole check finishes.",
                "-F",
                "draft=true",
            ],
            201,
        )?;
        let number = json(&created)?["number"]
            .as_u64()
            .ok_or_else(|| io::Error::other("GitHub returned no pull request number"))?;
        self.note(&format!("pr {number}"))?;
        let endpoint =
            format!("repos/{slug}/commits/{head}/check-runs?check_name=blackhole&per_page=100");
        let deadline = Instant::now() + self.timeout;
        let run = loop {
            let runs = json(&call(self.repo, "GET", &endpoint, &[], 200)?)?;
            if let Some(run) = runs["check_runs"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|run| {
                    run["app"]["id"].as_u64() == Some(ACTIONS_APP) && run["status"] == "completed"
                })
            {
                break run.clone();
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(Outcome::TimedOut(format!(
                    "canary: no finished blackhole check on {head} after {} seconds (check that the blackhole workflow runs on pull request #{number}, or rerun with a longer --timeout)",
                    self.timeout.as_secs()
                )));
            }
            thread::sleep(POLL.min(deadline - now));
        };
        let id = run["id"]
            .as_u64()
            .ok_or_else(|| io::Error::other("GitHub returned a check run without an id"))?;
        let log = call(
            self.repo,
            "GET",
            &format!("repos/{slug}/actions/jobs/{id}/logs"),
            &[],
            200,
        )?;
        Ok(Outcome::Judged {
            conclusion: run["conclusion"].as_str().unwrap_or_default().to_owned(),
            log: String::from_utf8_lossy(&log.body).into_owned(),
        })
    }
}

// The last summary line wins: diagnostics before it can quote any text a forged path holds.
fn gates(log: &str) -> Vec<&str> {
    log.lines()
        .filter_map(|line| line.split_once("blackhole ci: red (").map(|(_, rest)| rest))
        .next_back()
        .and_then(|rest| rest.split_once(')'))
        .map(|(gates, _)| gates.split(", ").collect())
        .unwrap_or_default()
}

fn cleanup_process(repo: &Repo, branch: &str) -> io::Result<Child> {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            CLEANUP,
            "blackhole-canary-cleanup",
            &repo.host,
            &repo.slug,
            branch,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn()
}

/// Pushes scripted forgeries to a new branch, opens a draft pull request and reports which ones the blackhole check stopped.
///
/// Returns 0 when every forgery was stopped, 1 when one got through and 75 when no verdict came in time.
///
/// # Errors
/// Reports repository, git, GitHub and output failures, after the pull request and branch are cleaned up.
pub fn run(timeout: Duration, out: &mut impl Write) -> io::Result<u8> {
    let root = crate::files::root()?;
    env::set_current_dir(&root)?;
    let repo = github::origin(&root)?
        .ok_or_else(|| io::Error::other("origin names no GitHub repository"))?;
    let info = json(&call(
        &repo,
        "GET",
        &format!("repos/{}", repo.slug),
        &[],
        200,
    )?)?;
    let base = info["default_branch"]
        .as_str()
        .ok_or_else(|| io::Error::other("GitHub returned no default branch"))?;
    let url = format!("https://{}/{}.git", repo.host, repo.slug);
    git::output(remote_git().args([
        "fetch",
        "-q",
        "--no-tags",
        &url,
        &format!("refs/heads/{base}"),
    ]))?;
    let tip = git::commit("FETCH_HEAD")?;
    let list = forgeries(&tip)?;
    let (workflow, pushed_first) = list
        .split_last()
        .ok_or_else(|| io::Error::other("no forgeries"))?;
    let mut head = tip;
    for forgery in pushed_first {
        head = commit(&head, forgery)?;
    }
    let workflow_head = commit(&head, workflow)?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let branch = format!("blackhole-canary-{seconds}");

    let mut cleaner = cleanup_process(&repo, &branch)?;
    let mut pipe = cleaner
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("cleanup stdin"))?;
    let result = (|| {
        push(&url, &head, &branch)?;
        let mut run = Run {
            repo: &repo,
            branch: &branch,
            base,
            timeout,
            cleanup: &mut pipe,
        };
        run.note("branch")?;
        let refused = matches!(push(&url, &workflow_head, &branch)?, Push::Refused);
        if !refused {
            head = workflow_head;
        }
        let outcome = run.judge(&head, list.len())?;
        Ok::<_, io::Error>((refused, outcome))
    })();
    drop(pipe);
    let status = cleaner.wait()?;
    let manual = format!("close the canary pull request and delete branch {branch} by hand");
    let (refused, outcome) = result.map_err(|e| {
        if status.success() {
            e
        } else {
            io::Error::other(format!("{e}; cleanup failed, {manual}"))
        }
    })?;
    let code = report(out, &list, refused, &outcome)?;
    if !status.success() {
        return Err(io::Error::other(format!("cleanup failed; {manual}")));
    }
    writeln!(
        out,
        "Closed the canary pull request and deleted branch {branch}."
    )?;
    Ok(code)
}

// The workflow edit is last, and a refused push of it is GitHub stopping it.
fn report(
    out: &mut impl Write,
    forgeries: &[Forgery],
    refused: bool,
    outcome: &Outcome,
) -> io::Result<u8> {
    let (failed, log) = match outcome {
        Outcome::Judged { conclusion, log } => (conclusion == "failure", log.as_str()),
        Outcome::TimedOut(message) => {
            writeln!(out, "{message}")?;
            (false, "")
        }
    };
    let timed_out = matches!(outcome, Outcome::TimedOut(_));
    let named = gates(log);
    let mut code = if timed_out { 75 } else { 0 };
    for (index, forgery) in forgeries.iter().enumerate() {
        let verdict = if refused && index + 1 == forgeries.len() {
            "stopped by GitHub, which refused the push because gh's credential cannot change workflows"
        } else if timed_out {
            "no verdict"
        } else if failed && forgery.stopped(&named, log) {
            "stopped"
        } else {
            code = 1;
            "not stopped"
        };
        writeln!(out, "{}: {}: {verdict}", forgery.gate, forgery.what)?;
    }
    Ok(code)
}
