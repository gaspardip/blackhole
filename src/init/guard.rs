use crate::github::{self, ACTIONS_APP, CHECK, Failure, Repo, Response};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

/// Why the repository is not guarded, and the exit code that says who can fix it.
pub(super) struct Stop {
    pub(super) code: u8,
    pub(super) reason: String,
}

fn not(reason: impl Into<String>) -> Stop {
    Stop {
        code: 1,
        reason: reason.into(),
    }
}

impl From<Failure> for Stop {
    fn from(failure: Failure) -> Self {
        match &failure {
            Failure::NoSession => not(failure.to_string()),
            Failure::Unreachable(error) => Self {
                code: 75,
                reason: format!("cannot read GitHub ({error}; rerun init once GitHub answers)"),
            },
        }
    }
}

fn get(repo: &Repo, endpoint: &str, args: &[&str]) -> Result<Response, Stop> {
    Ok(github::api(&repo.host, "GET", endpoint, args)?)
}

fn unexpected(endpoint: &str, response: &Response) -> Stop {
    not(format!(
        "GitHub answered {endpoint} with HTTP {} (check that gh can read the repository)",
        response.status
    ))
}

fn json(endpoint: &str, response: &Response) -> Result<Value, Stop> {
    match (response.status, response.json()) {
        (200, Some(value)) => Ok(value),
        _ => Err(unexpected(endpoint, response)),
    }
}

enum Classic {
    Absent,
    Hidden,
    Rules(Value),
}

/// Reads GitHub for the conditions under which an agent cannot get a change past the judge.
pub(super) fn verdict(root: &Path, workflows: &[(&str, &[u8])]) -> Result<(), Stop> {
    let repo = github::origin(root)
        .map_err(|e| not(format!("cannot read the origin remote ({e})")))?
        .ok_or_else(|| {
            not("origin names no GitHub repository (add the GitHub repository as the origin remote)")
        })?;
    let user = get(&repo, "user", &[])?;
    let login = json("user", &user)?["login"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| unexpected("user", &user))?;
    let slug = &repo.slug;
    let endpoint = format!("repos/{slug}");
    let response = get(&repo, &endpoint, &[])?;
    if matches!(response.status, 403 | 404) {
        return Err(not(format!(
            "cannot read {slug} on GitHub (sign gh in as an account that can read it)"
        )));
    }
    let info = json(&endpoint, &response)?;
    let branch = info["default_branch"]
        .as_str()
        .ok_or_else(|| unexpected(&endpoint, &response))?;
    let encoded = github::encode(branch);
    for &workflow in workflows {
        on_branch(&repo, branch, &encoded, workflow)?;
    }
    let rulesets = required(&repo, branch, &encoded)?;
    credential(&repo, &login, &info, &user, rulesets)
}

fn on_branch(
    repo: &Repo,
    branch: &str,
    encoded: &str,
    (path, workflow): (&str, &[u8]),
) -> Result<(), Stop> {
    let slug = &repo.slug;
    let endpoint = format!("repos/{slug}/contents/{path}?ref={encoded}");
    let file = get(
        repo,
        &endpoint,
        &["-H", "Accept: application/vnd.github.raw"],
    )?;
    match file.status {
        404 => {
            return Err(not(format!(
                "{path} is not on {branch} (commit it and merge it into {branch})"
            )));
        }
        200 if file.body != workflow => {
            return Err(not(format!(
                "{path} on {branch} differs from the one init writes (merge the generated file into {branch})"
            )));
        }
        200 => {}
        _ => return Err(unexpected(&endpoint, &file)),
    }
    Ok(())
}

fn valid_rule(rule: &Value) -> bool {
    let Some(kind) = rule["type"].as_str() else {
        return false;
    };
    rule["ruleset_id"].is_u64()
        && (kind != "required_status_checks"
            || rule["parameters"]["required_status_checks"]
                .as_array()
                .is_some_and(|checks| {
                    checks.iter().all(|check| {
                        check["context"].is_string()
                            && (check["integration_id"].is_null()
                                || check["integration_id"].is_u64())
                    })
                }))
}

// The rulesets GitHub applies to the branch, once they require pull requests and the blackhole check from GitHub Actions.
fn required(repo: &Repo, branch: &str, encoded: &str) -> Result<BTreeSet<u64>, Stop> {
    let slug = &repo.slug;
    let mut page = 1;
    let mut rules = Vec::new();
    loop {
        let endpoint = format!("repos/{slug}/rules/branches/{encoded}?per_page=100&page={page}");
        let response = get(repo, &endpoint, &[])?;
        if response.status != 200 {
            return Err(unexpected(&endpoint, &response));
        }
        let mut entries = match response.json() {
            Some(Value::Array(entries)) if entries.iter().all(valid_rule) => entries,
            _ => {
                return Err(Failure::Unreachable(format!(
                    "{endpoint} returned a malformed rules list"
                ))
                .into());
            }
        };
        let last = entries.len() < 100;
        rules.append(&mut entries);
        if last {
            break;
        }
        page += 1;
    }
    let mut pull_requests = rules.iter().any(|rule| rule["type"] == "pull_request");
    // Each source that may report the check, and whether its rule makes a pull request merge only on the current tip.
    let mut sources: Vec<(Option<u64>, bool)> = rules
        .iter()
        .filter(|rule| rule["type"] == "required_status_checks")
        .flat_map(|rule| {
            let strict = rule["parameters"]["strict_required_status_checks_policy"] == true;
            rule["parameters"]["required_status_checks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|check| check["context"] == CHECK)
                .map(move |check| (check["integration_id"].as_u64(), strict))
        })
        .collect();
    let classic = if pull_requests && sources.contains(&(Some(ACTIONS_APP), true)) {
        Classic::Absent
    } else {
        let endpoint = format!("repos/{slug}/branches/{encoded}/protection");
        let response = get(repo, &endpoint, &[])?;
        match (response.status, response.json()) {
            (200, Some(value)) => Classic::Rules(value),
            (404, Some(value)) if value["message"] == "Branch not protected" => Classic::Absent,
            // GitHub shows classic protection only to admins.
            (403 | 404, _) => Classic::Hidden,
            _ => return Err(unexpected(&endpoint, &response)),
        }
    };
    if let Classic::Rules(protection) = &classic {
        pull_requests |= protection["required_pull_request_reviews"].is_object();
        let checks = &protection["required_status_checks"];
        let strict = checks["strict"] == true;
        sources.extend(
            checks["checks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|check| check["context"] == CHECK)
                .map(|check| (check["app_id"].as_u64(), strict)),
        );
    }
    let hidden = matches!(classic, Classic::Hidden);
    let unreadable = || {
        not(format!(
            "cannot read the branch protection of {branch} without admin rights (require pull requests and the blackhole check in a ruleset for {branch}, which gh can read)"
        ))
    };
    if !pull_requests {
        return Err(if hidden {
            unreadable()
        } else {
            not(format!(
                "{branch} does not require pull requests (add a ruleset for {branch} that requires a pull request before merging)"
            ))
        });
    }
    let actions: Vec<bool> = sources
        .iter()
        .filter(|(source, _)| *source == Some(ACTIONS_APP))
        .map(|&(_, strict)| strict)
        .collect();
    if actions.is_empty() {
        return Err(if !sources.is_empty() {
            not(format!(
                "{branch} accepts the blackhole check from a source other than GitHub Actions (set the check's source to GitHub Actions)"
            ))
        } else if hidden {
            unreadable()
        } else {
            not(format!(
                "{branch} does not require the blackhole check (require the blackhole check from GitHub Actions in a ruleset for {branch})"
            ))
        });
    }
    if !actions.contains(&true) {
        return Err(if hidden {
            unreadable()
        } else {
            not(format!(
                "{branch} does not require branches to be up to date before merging (require it for the blackhole check on {branch}, so a green judged on an older {branch} cannot merge)"
            ))
        });
    }
    Ok(rules
        .iter()
        .filter_map(|rule| rule["ruleset_id"].as_u64())
        .collect())
}

fn credential(
    repo: &Repo,
    login: &str,
    info: &Value,
    user: &Response,
    rulesets: BTreeSet<u64>,
) -> Result<(), Stop> {
    let slug = &repo.slug;
    match info["permissions"]["admin"].as_bool() {
        Some(false) => {}
        Some(true) => {
            return Err(not(format!(
                "gh is signed in as {login}, an admin of {slug} who can bypass or change the rules (sign gh in as an account without admin rights)"
            )));
        }
        None => {
            return Err(not(format!(
                "GitHub does not reveal whether {login} is an admin of {slug} (sign gh in as an account without admin rights)"
            )));
        }
    }
    for id in rulesets {
        let endpoint = format!("repos/{slug}/rulesets/{id}");
        let ruleset = json(&endpoint, &get(repo, &endpoint, &[])?)?;
        let name = ruleset["name"].as_str().unwrap_or("without a name");
        match ruleset["current_user_can_bypass"].as_str() {
            Some("never") => {}
            Some(_) => {
                return Err(not(format!(
                    "gh is signed in as {login}, who can bypass ruleset {name} (take {login} and its teams and roles off the ruleset's bypass list)"
                )));
            }
            None => {
                return Err(not(format!(
                    "GitHub does not reveal whether {login} can bypass ruleset {name} (take {login} off its bypass list and sign gh in with gh auth login)"
                )));
            }
        }
    }
    match user.header("x-oauth-scopes") {
        None => Err(not(
            "GitHub does not reveal the rights of gh's token, so it may rewrite workflows (sign gh in with gh auth login, whose token lists its scopes)",
        )),
        Some(scopes) if scopes.split(',').any(|scope| scope.trim() == "workflow") => Err(not(
            "gh's token holds the workflow scope, so it can rewrite the judge (run gh auth refresh --remove-scopes workflow)",
        )),
        Some(_) => Ok(()),
    }
}
