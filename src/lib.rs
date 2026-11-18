pub mod ci;
mod config;
pub mod engine;
mod files;
mod git;
pub mod ledger;
pub mod objections;
mod packs {
    pub(crate) mod ts {
        pub(crate) mod dictionary_types;
        pub(crate) mod function_types;
        pub(crate) mod no_chained_type_assertions;
        pub(crate) mod no_comments;
        pub(crate) mod no_explicit_any;
        pub(crate) mod no_known_value_widening;
        pub(crate) mod no_non_null_assertion;
        pub(crate) mod no_unknown_parameters;
        pub(crate) mod no_widen_then_assert;
    }
    pub(crate) mod core {
        pub(crate) mod no_suppression;
    }
}
pub mod verdict;

use std::{
    fmt::Write as _,
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
        replace(&scan.ledger, &ledger::write(&shrunk)?)?;
    }
    Ok(scan.over)
}

// A link in place of the directory would move the ledger and the objections outside the repository.
fn home(root: &Path) -> io::Result<PathBuf> {
    let home = root.join(".blackhole");
    match fs::symlink_metadata(&home) {
        Ok(meta) if !meta.is_dir() => Err(io::Error::other(".blackhole must be a directory")),
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(home),
    }
}

// The rename is the only write to the file, so a crash leaves the old one whole.
fn replace(path: &Path, contents: &str) -> io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{}.tmp", std::process::id()));
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
    let (root, mut names) = files::list(paths)?;
    let excludes =
        config::parse(&files::policy(&root.join("blackhole.toml"))?.unwrap_or_default())?;
    names.retain(|name| !excludes.is_match(name));
    let home = home(&root)?;
    // A repository without a ledger allows nothing, which is what an empty one says.
    let base = files::policy(&home.join("ledger"))?.map_or_else(
        || Ok(ledger::Counts::new()),
        |bytes| ledger::parse(&bytes, engine::registry()),
    )?;
    let mut findings = scan_files(names.iter().map(String::as_str), |name| {
        files::read(&root.join(name))
    })?;
    let actual = ledger::counts(&findings);
    let today = objections::Date::today()?;
    let objections = objections::parse(
        &files::policy(&home.join("objections"))?.unwrap_or_default(),
        today,
    )?;
    let over = ledger::excess(&actual, &objections::allowances(&base, &objections, today));
    let mut stale = ledger::excess(&base, &actual);
    if !paths.is_empty() {
        // The list is sorted. A line for a file outside the selection says nothing about this run.
        stale.retain(|(_, path), _| names.binary_search(path).is_ok());
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
    names: impl Iterator<Item = &'a str>,
    mut read: impl FnMut(&str) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<Vec<engine::Finding>> {
    let mut findings = Vec::new();
    for name in names {
        let bytes = read(name).map_err(|e| io::Error::new(e.kind(), format!("{name:?}: {e}")))?;
        if let Some(bytes) = bytes {
            findings.extend(engine::run(engine::registry(), [(name, bytes.as_slice())]));
        }
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
pub fn object(rule: String, path: String, reason: Option<&str>, renew: bool) -> io::Result<()> {
    let _lock = files::lock()?;
    let dir = home(&files::root()?)?;
    let file = dir.join("objections");
    let bytes = files::policy(&file)?.unwrap_or_default();
    let today = objections::Date::today()?;
    let rows = objections::parse(&bytes, today)?;
    if !engine::can_allow(engine::registry(), &rule) {
        return Err(io::Error::other("rule cannot be objected to"));
    }
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
        .map_err(io::Error::other)?;
    }
    match fs::create_dir(&dir) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    replace(&file, &contents)
}
