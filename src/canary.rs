use crate::{
    ci,
    engine::{self, Gate},
    files::{LEDGER_PATH, OBJECTIONS_PATH},
    git::{self, Batch, Tree},
    github::{self, ACTIONS_APP, CHECK, Repo, Response},
    init::workflow::{self, PATH as WORKFLOW_PATH},
    ledger,
};
use serde_json::Value;
use std::{
    env,
    io::{self, Write},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const POLL: Duration = Duration::from_secs(10);
const MARK: &str = "# blackhole canary forgery\n";
const FINDING: &str = "blackhole-canary-finding.py";
// Runs in its own process group with the canary's end of the pipe as input, so it cleans up after an interrupt too.
const CLEANUP: &str = r#"branch=
while read -r kind; do
  [ "$kind" != branch ] || branch=1
done
[ -n "$branch" ] || exit 0
status=0
prs=$(gh api --hostname "$1" --method GET --paginate --jq '.[].number' "repos/$2/pulls?head=${2%%/*}:$3&state=open") || status=1
for pr in $prs; do
  gh api --hostname "$1" --method PATCH -f state=closed "repos/$2/pulls/$pr" >/dev/null || status=1
done
if ! response=$(gh api --hostname "$1" --method DELETE -i "repos/$2/git/refs/heads/$3"); then
  case "$(printf '%s\n' "$response" | head -n 1) " in
    HTTP/*" 404 "*|HTTP/*" 422 "*) ;;
    *) status=1 ;;
  esac
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

fn call(
    repo: &Repo,
    method: &str,
    endpoint: &str,
    args: &[&str],
    want: u16,
) -> io::Result<Response> {
    let response = github::api(&repo.host, method, endpoint, args)
        .map_err(|failure| io::Error::other(failure.to_string()))?;
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

// Commits each forgery on top of the one before it and returns every commit, last forgery last.
fn commits(tip: &str, forgeries: &[&Forgery]) -> io::Result<Vec<String>> {
    let index = env::temp_dir().join(format!("blackhole-canary-{}.index", std::process::id()));
    let indexed = |args: &[&str]| {
        git::output(git::command().env("GIT_INDEX_FILE", &index).args(args)).and_then(text)
    };
    let result = (|| {
        indexed(&["read-tree", tip])?;
        let mut chain: Vec<String> = Vec::new();
        for forgery in forgeries {
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
            let parent = chain.last().map_or(tip, String::as_str);
            let message = format!("blackhole canary: {}", forgery.what);
            let commit = git::output(
                git::command()
                    .envs([
                        ("GIT_AUTHOR_NAME", "blackhole canary"),
                        ("GIT_AUTHOR_EMAIL", "canary@blackhole.invalid"),
                        ("GIT_COMMITTER_NAME", "blackhole canary"),
                        ("GIT_COMMITTER_EMAIL", "canary@blackhole.invalid"),
                    ])
                    .args(["commit-tree", &tree, "-p", parent, "-m", &message]),
            )
            .and_then(text)?;
            chain.push(commit);
        }
        Ok(chain)
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

// One forgery for each gate the base policy can turn red, and the workflow edit apart because GitHub may refuse its push.
fn forgeries(tip: &str) -> io::Result<(Vec<Forgery>, Forgery)> {
    let tree = Tree::load(tip)?;
    let mut batch = Batch::start()?;
    let mut counts = ci::ledger(&tree, &mut batch)?;
    let config = ci::configuration(&tree, &mut batch)?;
    let mut read = |path| tree.judge_file(path, &mut batch);
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
    for test in tree.paths().filter(|path| {
        crate::packs::ts::test_context::test_file_path(path) && !config.exclude.is_match(path)
    }) {
        if tree.historical_blob(test, &mut batch)?.is_some() {
            list.push(Forgery {
                gate: "tests",
                what: format!("delete {test}"),
                change: Change::Delete(test.to_owned()),
            });
            break;
        }
    }
    batch.finish()?;
    list.push(Forgery {
        gate: "findings",
        what: format!("add a finding in {FINDING}"),
        change: Change::Write(FINDING, b"# noqa\n".to_vec()),
    });
    let workflow = Forgery {
        gate: "protected",
        what: format!("edit {WORKFLOW_PATH}"),
        change: Change::Write(WORKFLOW_PATH, judge),
    };
    Ok((list, workflow))
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
        crate::diagnostic_text(&git::failure(&out).to_string())
    )))
}

struct Run<'a> {
    repo: &'a Repo,
    branch: &'a str,
    base: &'a str,
    timeout: Duration,
}

enum Outcome {
    Judged { conclusion: String, log: String },
    Unavailable(String),
}

impl Run<'_> {
    fn judge(&self, head: &str, count: usize) -> io::Result<Outcome> {
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
        let endpoint =
            format!("repos/{slug}/commits/{head}/check-runs?check_name={CHECK}&per_page=100");
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
                return Ok(Outcome::Unavailable(format!(
                    "canary: no finished blackhole check on {head} after {} seconds (check that the blackhole workflow runs on pull request #{number}, or rerun with a longer --timeout)",
                    self.timeout.as_secs()
                )));
            }
            thread::sleep(POLL.min(deadline - now));
        };
        let conclusion = run["conclusion"].as_str().unwrap_or("unknown");
        if !matches!(conclusion, "success" | "failure") {
            return Ok(Outcome::Unavailable(format!(
                "canary: the blackhole check concluded {}",
                crate::diagnostic_text(conclusion)
            )));
        }
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
            conclusion: conclusion.to_owned(),
            log: String::from_utf8_lossy(&log.body).into_owned(),
        })
    }
}

// The last summary line wins: diagnostics before it can quote any text a forged path holds.
fn gates(log: &str) -> Vec<&str> {
    log.lines()
        .filter_map(|line| {
            let text = line
                .split_once(' ')
                .filter(|(stamp, _)| {
                    stamp.as_bytes().get(10) == Some(&b'T') && stamp.ends_with('Z')
                })
                .map_or(line, |(_, rest)| rest);
            text.strip_prefix(ci::RED)?.strip_suffix(')')
        })
        .next_back()
        .filter(|gates| !gates.is_empty())
        .map(|gates| gates.split(", ").collect())
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
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn()
}

/// Pushes scripted forgeries to a new branch, opens a draft pull request and reports which ones the blackhole check stopped.
///
/// Returns 0 when every forgery was stopped, 1 when one got through and 75 when there is no verdict.
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
    let (list, workflow) = forgeries(&tip)?;
    let mut chain = commits(&tip, &list.iter().chain([&workflow]).collect::<Vec<_>>())?;
    let (Some(workflow_head), Some(mut head)) = (chain.pop(), chain.pop()) else {
        return Err(io::Error::other("no forgeries"));
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let branch = format!("blackhole-canary-{nanos}-{}", std::process::id());

    let mut cleaner = cleanup_process(&repo, &branch)?;
    let mut pipe = cleaner
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("cleanup stdin"))?;
    let result = (|| {
        writeln!(pipe, "branch")?;
        push(&url, &head, &branch)?;
        let run = Run {
            repo: &repo,
            branch: &branch,
            base,
            timeout,
        };
        let refused = matches!(push(&url, &workflow_head, &branch)?, Push::Refused);
        if !refused {
            head = workflow_head;
        }
        let outcome = run.judge(&head, list.len() + 1)?;
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
    let code = report(out, &list, &workflow, refused, &outcome)?;
    if !status.success() {
        return Err(io::Error::other(format!("cleanup failed; {manual}")));
    }
    writeln!(
        out,
        "Closed the canary pull request and deleted branch {branch}."
    )?;
    Ok(code)
}

// A refused push of the workflow edit is GitHub stopping it.
fn report(
    out: &mut impl Write,
    forgeries: &[Forgery],
    workflow: &Forgery,
    refused: bool,
    outcome: &Outcome,
) -> io::Result<u8> {
    let (failed, log) = match outcome {
        Outcome::Judged { conclusion, log } => (conclusion == "failure", log.as_str()),
        Outcome::Unavailable(message) => {
            writeln!(out, "{message}")?;
            (false, "")
        }
    };
    let named = gates(log);
    let missing_summary = failed && named.is_empty();
    if missing_summary {
        writeln!(
            out,
            "canary: the failed blackhole check did not emit a judge verdict"
        )?;
    }
    let no_verdict = missing_summary || matches!(outcome, Outcome::Unavailable(_));
    let mut code = if no_verdict { 75 } else { 0 };
    let all = forgeries.iter().map(|f| (f, false));
    for (forgery, is_workflow) in all.chain([(workflow, true)]) {
        let verdict = if no_verdict {
            "no verdict"
        } else if refused && is_workflow {
            "stopped by GitHub, which refused the push because gh's credential cannot change workflows"
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
