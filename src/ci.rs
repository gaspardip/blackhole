use crate::{
    config,
    engine::{self, DiffCtx, DiffFindings, Run},
    files::{LEDGER_PATH, OBJECTIONS_PATH},
    git::{self, Batch, Tree},
    ledger,
    objections::{self, Date},
    verdict::{self, Approvals, Inputs, Verdict},
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

fn configuration(bytes: Option<Vec<u8>>) -> io::Result<config::Config> {
    config::parse(&bytes.unwrap_or_default())
}

// A red failure leaves the gates that do not depend on it; any other failure ends the judgment.
fn red_or<T>(result: io::Result<T>) -> io::Result<io::Result<T>> {
    match result {
        Err(e) if crate::mark(&e) != Some(crate::Fault::Red) => Err(e),
        result => Ok(result),
    }
}

// Only missing, invalid or short evidence is the author's; an outage is no verdict.
fn evidence(result: io::Result<()>) -> io::Result<io::Result<()>> {
    match result {
        Err(e) if crate::fault(&e) == crate::Fault::NoVerdict => Err(e),
        result => Ok(result),
    }
}

// Removals found before a failure stay, so an unreadable file cannot hide a deleted test.
fn diff_rules(ctx: &mut DiffCtx<'_>) -> (DiffFindings, io::Result<()>) {
    let mut findings = DiffFindings::default();
    let result = engine::registry()
        .iter()
        .try_for_each(|rule| match rule.run {
            Run::Diff(check) => check(ctx, &mut findings),
            _ => Ok(()),
        });
    engine::sort_findings(&mut findings.removals);
    (findings, result)
}

fn evaluate(
    base: &str,
    head: &str,
    approved: bool,
    today: Date,
    artifacts: Option<&Path>,
) -> io::Result<Verdict> {
    if !git::ancestor(base, head)? {
        return Ok(Verdict::behind());
    }
    // The pull request cannot fix the base, so a base it cannot read is no verdict.
    let base_tree = Tree::load(base).map_err(crate::no_verdict)?;
    let head_tree = Tree::load(head)?;
    let mut batch = Batch::start()?;
    let mut base_policy = |path| base_tree.policy(path, &mut batch);
    let base_config = base_policy("blackhole.toml")
        .and_then(configuration)
        .map_err(crate::no_verdict)?;
    let base_ledger = base_policy(LEDGER_PATH)
        .and_then(|bytes| ledger::load(bytes.as_deref()))
        .map_err(crate::no_verdict)?;
    let base_rows = base_policy(OBJECTIONS_PATH)
        .and_then(|bytes| objections::parse(&bytes.unwrap_or_default(), today))
        .map_err(crate::no_verdict)?;
    // The head's policy is judged whether or not it is approved, and applied only once it is.
    let head_config =
        red_or(head_tree.policy("blackhole.toml", &mut batch))?.and_then(configuration);
    let head_ledger = red_or(head_tree.policy(LEDGER_PATH, &mut batch))?
        .and_then(|bytes| ledger::load(bytes.as_deref()));
    let head_rows = red_or(head_tree.policy(OBJECTIONS_PATH, &mut batch))?
        .and_then(|bytes| objections::parse(&bytes.unwrap_or_default(), today));
    let scan_config = match &head_config {
        Ok(config) if approved => config,
        _ => &base_config,
    };
    let excludes = &scan_config.exclude;
    let changes = git::changes(base, head)?;
    let mut ctx = DiffCtx {
        base: &base_tree,
        head: &head_tree,
        changes: &changes,
        renames: &verdict::renames(&changes),
        base_excludes: &base_config.exclude,
        head_excludes: excludes,
        approved,
        batch: &mut batch,
    };
    let (diff, result) = diff_rules(&mut ctx);
    let findings = match red_or(result)? {
        Ok(()) => red_or(crate::scan_files(
            scan_config,
            diff.diagnostics,
            head_tree.paths(),
            &mut *ctx.batch,
            |batch| head_tree.repository_paths(batch),
            |batch, path| head_tree.read(path, batch),
        ))?,
        Err(e) => Err(e),
    };
    let removals = diff.removals;
    let mut verdict = verdict::judge_with_guard(
        &Inputs {
            approved,
            today,
            excludes,
            changes: ctx.changes,
            base_ledger: &base_ledger,
            head_ledger: head_ledger.as_ref(),
            base_rows: &base_rows,
            head_rows: head_rows.as_ref(),
            findings: findings.as_deref(),
            test_removals: &removals,
        },
        &base_config.guard,
        ctx.renames,
    )?;
    if let Err(e) = &head_config {
        verdict
            .gates
            .insert(verdict::unreadable_or(e, "configuration"));
        writeln!(verdict.lines, "{e}")?;
    }
    if let Some(lcov) = &base_config.lcov {
        let delta = red_or(crate::coverage::changed(
            &changes,
            &base_config.exclude,
            &base_tree,
            &head_tree,
            &mut batch,
        ))?;
        match delta {
            Ok(delta) => {
                if !delta.is_empty()
                    && let Err(e) = evidence(crate::coverage::check(&delta, artifacts, head, lcov))?
                {
                    verdict.gates.insert("coverage");
                    writeln!(verdict.lines, "coverage: {e}")?;
                }
            }
            Err(e) => {
                verdict.gates.insert("unreadable");
                writeln!(verdict.lines, "{e}")?;
            }
        }
    }
    batch.finish()?;
    Ok(verdict)
}

/// Which move clears a judgment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Green,
    /// The author fixes it.
    Red,
    /// Only a person's approval clears it.
    Waiting,
}

// An approval of the head clears these gates, and nothing the author does can.
const WAITING: [&str; 2] = ["protected", "tests"];

/// Judges two commits against the repository policy.
/// # Errors
/// Returns an error when git, base policy, approval data or output is unavailable.
pub fn run(
    base: &str,
    head: &str,
    approvals: Option<&Path>,
    today: Option<Date>,
    out: &mut impl Write,
) -> io::Result<State> {
    run_with_coverage(base, head, approvals, today, None, out)
}

/// Judges commits using LCOV supplied by the trusted caller for the resolved head.
/// # Errors
/// Returns an error when git, base policy, approval data or output is unavailable.
pub fn run_with_coverage(
    base: &str,
    head: &str,
    approvals: Option<&Path>,
    today: Option<Date>,
    artifacts: Option<&Path>,
    out: &mut impl Write,
) -> io::Result<State> {
    // The judge reads fetched commits, so a failure nobody marked is the fetch or the machine, not the change.
    judge(base, head, approvals, today, artifacts, out).map_err(|e| {
        if crate::mark(&e).is_some() {
            e
        } else {
            crate::no_verdict(e)
        }
    })
}

fn judge(
    base: &str,
    head: &str,
    approvals: Option<&Path>,
    today: Option<Date>,
    artifacts: Option<&Path>,
    out: &mut impl Write,
) -> io::Result<State> {
    let base = git::commit(base)?;
    let head = git::commit(head)?;
    let approved = match approvals {
        Some(path) => {
            let bytes = fs::read(path).map_err(|e| {
                // The caller named the file, so only the machine running out of something is an outage.
                if crate::fault(&e) == crate::Fault::NoVerdict {
                    e
                } else {
                    crate::usage(format!("{}: {e}", path.display()))
                }
            })?;
            serde_json::from_slice::<Approvals>(&bytes)
                .map_err(|e| crate::usage(format!("{}: {e}", path.display())))?
                .approved(&head)
        }
        None => false,
    };
    let result = match evaluate(
        &base,
        &head,
        approved,
        today.map_or_else(Date::today, Ok)?,
        artifacts,
    ) {
        Err(e) if crate::mark(&e) == Some(crate::Fault::Red) => Verdict::unreadable(&e),
        result => result?,
    };
    out.write_all(&result.lines)?;
    let (waiting, red): (Vec<_>, Vec<_>) = result
        .gates
        .into_iter()
        .partition(|gate| WAITING.contains(gate));
    let (waiting, red) = (waiting.join(", "), red.join(", "));
    let state = if !red.is_empty() {
        let pending = if waiting.is_empty() {
            String::new()
        } else {
            format!("; waiting ({waiting})")
        };
        writeln!(out, "blackhole ci: red ({red}){pending}")?;
        State::Red
    } else if !waiting.is_empty() {
        writeln!(
            out,
            "blackhole ci: waiting ({waiting}): a person with write access who wrote none of the change approves the head"
        )?;
        State::Waiting
    } else {
        writeln!(out, "blackhole ci: green")?;
        State::Green
    };
    Ok(state)
}
