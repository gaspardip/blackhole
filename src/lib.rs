pub mod engine;
pub mod ledger;
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
    for (key, allowed) in &scan.stale {
        let (rule, path) = key;
        let found = scan.actual.get(key).copied().unwrap_or(0);
        writeln!(
            output,
            "{path} {rule}: ledger allows {allowed}, found {found}; run blackhole ledger"
        )?;
    }
    Ok(scan.over || !scan.stale.is_empty())
}

/// Regenerates the repository ledger within its existing allowances.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn regenerate(output: &mut impl Write) -> io::Result<bool> {
    let scan = scan(&[], output)?;
    if !scan.over && !scan.stale.is_empty() {
        replace(&scan.ledger, &scan.actual)?;
    }
    Ok(scan.over)
}

// A repository without a ledger allows nothing, which is what an empty one says.
fn read_ledger(path: &Path) -> io::Result<ledger::Counts> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => ledger::parse(&fs::read(path)?, engine::registry()),
        Ok(_) => Err(io::Error::other(".blackhole/ledger must be a regular file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(ledger::Counts::new()),
        Err(e) => Err(e),
    }
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
    actual: ledger::Counts,
    over: bool,
    stale: ledger::Counts,
}

fn scan(paths: &[PathBuf], output: &mut impl Write) -> io::Result<Scan> {
    let (root, names) = files::list(paths)?;
    let ledger = root.join(".blackhole/ledger");
    let base = read_ledger(&ledger)?;
    let rules = engine::registry();
    let mut findings = Vec::new();
    for name in &names {
        let bytes = fs::read(root.join(name))
            .map_err(|e| io::Error::new(e.kind(), format!("{name:?}: {e}")))?;
        findings.extend(engine::run(rules, [(name.as_str(), bytes.as_slice())]));
    }
    let actual = ledger::counts(&findings);
    let over = ledger::excess(&actual, &base);
    let mut stale = ledger::excess(&base, &actual);
    if !paths.is_empty() {
        // The list is sorted. A line for a file outside the selection says nothing about this run.
        stale.retain(|(_, path), _| names.binary_search(path).is_ok());
    }
    findings.retain(|f| over.contains_key(&(f.rule.to_owned(), f.path.clone())));
    report(&findings, output)?;
    Ok(Scan {
        ledger,
        actual,
        over: !over.is_empty(),
        stale,
    })
}

fn report(findings: &[engine::Finding], output: &mut impl Write) -> io::Result<()> {
    for f in findings {
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
