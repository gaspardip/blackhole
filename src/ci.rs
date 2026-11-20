use crate::{
    config,
    git::{self, Batch, Tree},
    ledger::{self, Counts},
    objections::{self, Date, Rows},
    verdict::{self, Approvals, Inputs, Verdict},
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

fn ledger(tree: &Tree, batch: &mut Batch) -> io::Result<Counts> {
    ledger::load(tree.judge_file(".blackhole/ledger", batch)?)
}
fn configuration(tree: &Tree, batch: &mut Batch) -> io::Result<config::Config> {
    config::parse(
        &tree
            .judge_file("blackhole.toml", batch)?
            .unwrap_or_default(),
    )
}
fn rows(tree: &Tree, batch: &mut Batch, day: Date) -> io::Result<Rows> {
    objections::parse(
        &tree
            .judge_file(".blackhole/objections", batch)?
            .unwrap_or_default(),
        day,
    )
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
    let base_tree = Tree::load(base)?;
    let head_tree = Tree::load(head)?;
    let mut batch = Batch::start()?;
    let base_config = configuration(&base_tree, &mut batch)?;
    let head_config;
    let excludes = if approved {
        head_config = configuration(&head_tree, &mut batch)?;
        &head_config.exclude
    } else {
        &base_config.exclude
    };
    let base_ledger = ledger(&base_tree, &mut batch)?;
    let base_rows = rows(&base_tree, &mut batch, today)?;
    let head_ledger = ledger(&head_tree, &mut batch);
    let head_rows = rows(&head_tree, &mut batch, today);
    let changes = git::changes(base, head)?;
    let findings = crate::scan_files(
        head_tree.paths().filter(|p| !excludes.is_match(p)),
        |path| head_tree.read(path, &mut batch),
    )?;
    let mut verdict = verdict::judge_with_guard(
        &Inputs {
            approved,
            today,
            excludes,
            changes: &changes,
            base_ledger: &base_ledger,
            head_ledger: head_ledger.as_ref(),
            base_rows: &base_rows,
            head_rows: head_rows.as_ref(),
            findings: &findings,
        },
        &base_config.guard,
    )?;
    if let Some(lcov) = &base_config.lcov {
        let delta = crate::coverage::changed(
            &changes,
            &base_config.exclude,
            &base_tree,
            &head_tree,
            &mut batch,
        )?;
        if !delta.is_empty()
            && let Err(e) = crate::coverage::check(&delta, artifacts, head, lcov)
        {
            verdict.gates.insert("coverage");
            writeln!(verdict.lines, "coverage: {e}")?;
        }
    }
    batch.finish()?;
    Ok(verdict)
}

/// Judges two commits against the repository policy.
/// # Errors
/// Returns an error when git, base policy, approval data or output is unavailable.
pub fn run(
    base: &str,
    head: &str,
    approvals: Option<&Path>,
    today: Option<Date>,
    out: &mut impl Write,
) -> io::Result<bool> {
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
) -> io::Result<bool> {
    let base = git::commit(base)?;
    let head = git::commit(head)?;
    let approved = match approvals {
        Some(path) => serde_json::from_slice::<Approvals>(&fs::read(path)?)
            .map_err(io::Error::other)?
            .approved(&head),
        None => false,
    };
    let result = evaluate(
        &base,
        &head,
        approved,
        today.map_or_else(Date::today, Ok)?,
        artifacts,
    )?;
    out.write_all(&result.lines)?;
    if result.gates.is_empty() {
        return Ok(false);
    }
    writeln!(
        out,
        "blackhole ci: red ({})",
        result.gates.into_iter().collect::<Vec<_>>().join(", ")
    )?;
    Ok(true)
}
