use crate::{
    config,
    engine::{self, Finding},
    git::{self, Batch, Tree},
    ledger::{self, Counts},
    objections::{self, Date, Rows},
    packs::ts::{no_test_removal, test_context::test_file_path},
    verdict::{self, Approvals, Change, Inputs, Verdict},
};
use globset::GlobSet;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
};

fn ledger(tree: &Tree, batch: &mut Batch) -> io::Result<Counts> {
    ledger::load(tree.judge_file(".blackhole/ledger", batch)?.as_deref())
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

fn renamed_test_diagnostics(
    head: &Tree,
    changes: &[Change],
    excludes: &GlobSet,
    batch: &mut Batch,
) -> io::Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for change in changes {
        if let Some(old) = change.old()
            && test_file_path(old)
            && !excludes.is_match(old)
            && !excludes.is_match(&change.path)
            && engine::source_type(old).is_some()
            && engine::source_type(&change.path).is_none()
            && let Some(bytes) = head.read(&change.path, batch)?
        {
            findings.extend(engine::parse_diagnostics(old, &change.path, &bytes));
        }
    }
    Ok(findings)
}

fn test_removals(
    base: &Tree,
    head: &Tree,
    changes: &[Change],
    excludes: &GlobSet,
    batch: &mut Batch,
) -> io::Result<Vec<Finding>> {
    let renames: BTreeMap<_, _> = changes
        .iter()
        .filter_map(|c| Some((c.old()?, c.path.as_str())))
        .collect();
    let mut findings = Vec::new();
    for old in base
        .paths()
        .filter(|p| test_file_path(p) && !excludes.is_match(p))
    {
        let Some(before) = base.blob(old, batch)? else {
            continue;
        };
        let new = renames.get(old).copied().unwrap_or(old);
        let after = if excludes.is_match(new) {
            None
        } else {
            head.blob(new, batch)?
        };
        let Some(after) = after else {
            findings.push(no_test_removal::deleted(old));
            continue;
        };
        if before != after || old != new {
            let before = batch.read(before)?;
            let after = batch.read(after)?;
            findings.extend(no_test_removal::compare(old, new, &before, &after)?);
        }
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(findings)
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
    let mut findings = crate::scan_files(
        head_tree.paths().filter(|p| !excludes.is_match(p)),
        |path| head_tree.read(path, &mut batch),
    )?;
    let rename_findings = renamed_test_diagnostics(&head_tree, &changes, excludes, &mut batch)?;
    if !rename_findings.is_empty() {
        findings.extend(rename_findings);
        findings.sort_by(|a, b| (&a.path, a.line, a.col).cmp(&(&b.path, b.line, b.col)));
    }
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
    if !approved {
        verdict.test_removals(&test_removals(
            &base_tree, &head_tree, &changes, excludes, &mut batch,
        )?)?;
    }
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
