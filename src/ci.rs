use crate::{
    config, engine,
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
    tree.judge_file(".blackhole/ledger", batch)?.map_or_else(
        || Ok(Counts::new()),
        |b| ledger::parse(&b, engine::registry()),
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
fn evaluate(base: &str, head: &str, approved: bool, today: Date) -> io::Result<Verdict> {
    if !git::ancestor(base, head)? {
        return Ok(Verdict::behind());
    }
    let base_tree = Tree::load(base)?;
    let head_tree = Tree::load(head)?;
    let mut batch = Batch::start()?;
    let mut excludes = config::parse(
        &base_tree
            .judge_file("blackhole.toml", &mut batch)?
            .unwrap_or_default(),
    )?;
    if approved {
        excludes = config::parse(
            &head_tree
                .judge_file("blackhole.toml", &mut batch)?
                .unwrap_or_default(),
        )?;
    }
    let base_ledger = ledger(&base_tree, &mut batch)?;
    let base_rows = rows(&base_tree, &mut batch, today)?;
    let head_ledger = ledger(&head_tree, &mut batch);
    let head_rows = rows(&head_tree, &mut batch, today);
    let changes = git::changes(base, head)?;
    let findings = crate::scan_files(
        head_tree
            .blobs()
            .filter(|(p, _)| !excludes.is_match(p))
            .map(|(p, id)| Ok((p, batch.read(id)?))),
    )?;
    batch.finish()?;
    verdict::judge(&Inputs {
        ancestor: true,
        approved,
        today,
        excludes: &excludes,
        changes: &changes,
        base_ledger: &base_ledger,
        head_ledger: head_ledger.as_ref(),
        base_rows: &base_rows,
        head_rows: head_rows.as_ref(),
        findings: &findings,
    })
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
    let base = git::commit(base)?;
    let head = git::commit(head)?;
    let approved = approvals
        .map(|path| -> io::Result<bool> {
            let reviews: Approvals =
                serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
            Ok(reviews.approved(&head))
        })
        .transpose()?
        .unwrap_or(false);
    let result = evaluate(&base, &head, approved, today.map_or_else(Date::today, Ok)?)?;
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
