pub mod ci;
mod git;
pub mod verdict;
use std::fmt::Write as _;
mod config;
pub mod engine;
pub mod ledger;
pub mod objections;
mod packs {
    pub(crate) mod core {
        pub(crate) mod no_suppression;
    }
}

mod files;

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Checks the selected files and returns whether any rule was violated.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn check(paths: &[PathBuf], output: &mut impl Write) -> io::Result<bool> {
    let scan = scan(paths, output)?;
    report_stale(&scan.stale, &scan.actual, output)?;
    Ok(scan.over || !scan.stale.is_empty())
}

fn report_stale(
    stale: &ledger::Counts,
    actual: &ledger::Counts,
    output: &mut impl Write,
) -> io::Result<()> {
    for (key, allowed) in stale {
        let (rule, path) = key;
        let found = actual.get(key).copied().unwrap_or(0);
        writeln!(
            output,
            "{path} {rule}: ledger allows {allowed}, found {found}; run blackhole ledger"
        )?;
    }
    Ok(())
}

/// Regenerates the repository ledger within its existing allowances.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn regenerate(output: &mut impl Write) -> io::Result<bool> {
    let _lock = files::lock()?;
    let scan = scan(&[], output)?;
    if !scan.over && !scan.stale.is_empty() {
        let mut shrunk = scan.base;
        shrunk.retain(|key, n| {
            *n = (*n).min(scan.actual.get(key).copied().unwrap_or(0));
            *n > 0
        });
        replace(&scan.ledger, &shrunk)?;
    }
    Ok(scan.over)
}

// A repository without a ledger allows nothing, which is what an empty one says.
// A link at either level would point the ledger at a file outside the repository.
fn read_ledger(home: &Path) -> io::Result<ledger::Counts> {
    let ledger = home.join("ledger");
    for (path, directory) in [(home, true), (ledger.as_path(), false)] {
        match fs::symlink_metadata(path) {
            Ok(meta) if directory && meta.is_dir() || !directory && meta.is_file() => {}
            Ok(_) => {
                return Err(io::Error::other(
                    ".blackhole must be a directory and .blackhole/ledger a regular file",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ledger::Counts::new()),
            Err(e) => return Err(e),
        }
    }
    ledger::parse(&fs::read(ledger)?, engine::registry())
}

// The rename is the only write to the ledger, so a crash leaves the old one whole.
fn replace(path: &Path, actual: &ledger::Counts) -> io::Result<()> {
    let contents = ledger::write(actual)?;
    let temp = path.with_file_name(format!("ledger.{}.tmp", std::process::id()));
    fs::File::create_new(&temp)?.write_all(contents.as_bytes())?;
    fs::rename(temp, path)
}

struct Scan {
    ledger: PathBuf,
    base: ledger::Counts,
    actual: ledger::Counts,
    over: bool,
    stale: ledger::Counts,
}

fn scan(paths: &[PathBuf], output: &mut impl Write) -> io::Result<Scan> {
    let (root, mut names, selection) = files::list(paths)?;
    let excludes = config::parse(&config::read_optional(&root.join("blackhole.toml"))?)?;
    names.retain(|name| !excludes.is_match(name));
    let home = root.join(".blackhole");
    let base = read_ledger(&home)?;
    let mut findings = scan_files(names.iter().filter_map(|name| {
        files::read(&root.join(name))
            .map_err(|e| io::Error::new(e.kind(), format!("{name:?}: {e}")))
            .map(|bytes| bytes.map(|bytes| (name.as_str(), bytes)))
            .transpose()
    }))?;
    let actual = ledger::counts(&findings);
    let today = objections::Date::today()?;
    let objections = objections::parse(
        &config::read_optional(&root.join(".blackhole/objections"))?,
        today,
    )?;
    let over = ledger::excess(&actual, &objections::allowances(&base, &objections, today));
    let mut stale = ledger::excess(&base, &actual);
    if !selection.is_empty() {
        // A line for a file outside the selection says nothing about this run. A file that is gone is still inside it.
        stale.retain(|(_, path), _| {
            let file = root.join(path);
            selection.iter().any(|chosen| file.starts_with(chosen))
        });
    }
    findings.retain(|f| over.contains_key(&(f.rule.to_owned(), f.path.clone())));
    report(&findings, output)?;
    Ok(Scan {
        ledger: home.join("ledger"),
        actual,
        base,
        over: !over.is_empty(),
        stale,
    })
}

fn scan_files<'a>(
    files: impl Iterator<Item = io::Result<(&'a str, Vec<u8>)>>,
) -> io::Result<Vec<engine::Finding>> {
    let mut findings = Vec::new();
    for file in files {
        let (path, bytes) = file?;
        findings.extend(engine::run(engine::registry(), [(path, bytes.as_slice())]));
    }
    Ok(findings)
}

fn report<'a>(
    findings: impl IntoIterator<Item = &'a engine::Finding>,
    output: &mut impl Write,
) -> io::Result<()> {
    let findings: Vec<_> = findings.into_iter().collect();
    for f in &findings {
        write!(output, "{}:{}:{} {}", f.path, f.line, f.col, f.rule)?;
        if let Some(note) = &f.note {
            write!(output, " {note}")?;
        }
        writeln!(output)?;
    }
    for rule in engine::registry() {
        if findings.iter().any(|f| f.rule == rule.id) {
            writeln!(output, "{} {}", rule.id, rule.fix)?;
        }
    }
    Ok(())
}

/// Files or renews an objection against the working tree.
/// # Errors
/// Rejects invalid objections, absent overages and repeated renewals.
pub fn object(rule: String, path: String, reason: Option<&str>, renew: bool) -> io::Result<bool> {
    let (root, _) = files::list(&[])?;
    let dir = root.join(".blackhole");
    fs::create_dir_all(&dir)?;
    let lock = fs::File::open(&dir)?;
    lock.lock()?;
    let file = dir.join("objections");
    let bytes = config::read_optional(&file)?;
    let today = objections::Date::today()?;
    let rows = objections::parse(&bytes, today)?;
    let key = (rule, path);
    let mut contents = String::from_utf8(bytes).map_err(io::Error::other)?;
    if renew {
        if reason.is_some() {
            return Err(io::Error::other("renewal takes no reason"));
        }
        let row = rows
            .get(&key)
            .ok_or_else(|| io::Error::other("no objection"))?;
        if row.renewal.is_some() {
            return Err(io::Error::other("already renewed once"));
        }
        let prefix = format!("{}\t{}\t", key.0, key.1);
        contents = contents
            .split_inclusive('\n')
            .map(|line| {
                if line.starts_with(&prefix) {
                    format!(
                        "{prefix}{}\t{}\t{today}\t{}\n",
                        row.extra, row.filed, row.reason
                    )
                } else {
                    line.to_owned()
                }
            })
            .collect();
    } else {
        if rows.contains_key(&key) {
            return Err(io::Error::other("duplicate objection"));
        }
        let reason = reason
            .filter(|s| !s.trim().is_empty() && !s.contains(['\r', '\n', '\t']))
            .ok_or_else(|| io::Error::other("invalid reason"))?;
        let scan = scan(&[], &mut io::sink())?;
        let extra = scan
            .actual
            .get(&key)
            .copied()
            .unwrap_or(0)
            .saturating_sub(scan.base.get(&key).copied().unwrap_or(0));
        if extra == 0 || ledger::invalid_path(&key.1) {
            return Err(io::Error::other("no representable overage"));
        }
        writeln!(
            contents,
            "{}\t{}\t{extra}\t{today}\t-\t{reason}",
            key.0, key.1
        )
        .unwrap();
    }
    let temp = dir.join(format!("objections.{}.tmp", std::process::id()));
    fs::File::create_new(&temp)?.write_all(contents.as_bytes())?;
    fs::rename(temp, file)?;
    Ok(false)
}
