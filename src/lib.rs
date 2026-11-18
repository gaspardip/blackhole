pub mod engine;
pub mod ledger;
mod packs {
    pub(crate) mod core {
        pub(crate) mod no_suppression;
    }
}

mod files;

use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const MISPLACED: &str = ".blackhole must be a directory and .blackhole/ledger a regular file";

/// Checks the selected files and returns whether any rule was violated.
///
/// Leaves the process inside the repository's `.blackhole` directory when there is one.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn check(paths: &[PathBuf], output: &mut impl Write) -> io::Result<bool> {
    let scan = scan(&files::root()?, paths, output)?;
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
/// Leaves the process inside the repository's `.blackhole` directory when there is one.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn regenerate(output: &mut impl Write) -> io::Result<bool> {
    let root = files::root()?;
    let _lock = files::lock(&root)?;
    let scan = scan(&root, &[], output)?;
    if !scan.over && !scan.stale.is_empty() {
        replace(&scan.actual)?;
    }
    Ok(scan.over)
}

// A repository without a ledger allows nothing, which is what an empty one says.
// A link at either level would point the ledger at a file outside the repository.
fn read_ledger(home: &Path) -> io::Result<ledger::Counts> {
    if !(present(home, fs::Metadata::is_dir)?
        && enter(home)?
        && present("ledger", fs::Metadata::is_file)?)
    {
        return Ok(ledger::Counts::new());
    }
    ledger::parse(&fs::read("ledger")?, engine::registry())
}

fn present(path: impl AsRef<Path>, kind: fn(&fs::Metadata) -> bool) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if kind(&meta) => Ok(true),
        Ok(_) => Err(io::Error::other(MISPLACED)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

// The working directory holds the directory itself, so a link swapped in at its path later cannot move the ledger.
// A link swapped in before the move shows as a working directory that is somewhere else.
fn enter(home: &Path) -> io::Result<bool> {
    match env::set_current_dir(home) {
        Ok(()) if fs::canonicalize(".")? == home => Ok(true),
        Ok(()) => Err(io::Error::other(MISPLACED)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

// The rename is the only write to the ledger, so a crash leaves the old one whole.
// Both names are relative: a stale row means read_ledger entered the directory.
fn replace(actual: &ledger::Counts) -> io::Result<()> {
    let contents = ledger::write(actual)?;
    let temp = format!("ledger.{}.tmp", std::process::id());
    fs::File::create_new(&temp)?.write_all(contents.as_bytes())?;
    fs::rename(temp, "ledger")
}

struct Scan {
    actual: ledger::Counts,
    over: bool,
    stale: ledger::Counts,
}

fn scan(root: &Path, paths: &[PathBuf], output: &mut impl Write) -> io::Result<Scan> {
    // The selection is resolved before read_ledger moves the working directory.
    let (names, selection) = files::list(root, paths)?;
    let base = read_ledger(&root.join(".blackhole"))?;
    let under = |path: &str, chosen: &Path| root.join(path).starts_with(chosen);
    // A path with no file left is still a selection while the ledger has a row under it.
    for chosen in &selection {
        let rows = base.keys().map(|(_, path)| path);
        if !names.iter().chain(rows).any(|path| under(path, chosen)) {
            return Err(io::Error::other(format!(
                "no files match {}",
                chosen.display().to_string().escape_default()
            )));
        }
    }
    let rules = engine::registry();
    let mut findings = Vec::new();
    for name in &names {
        let bytes = files::read(&root.join(name))
            .map_err(|e| io::Error::new(e.kind(), format!("{name:?}: {e}")))?;
        if let Some(bytes) = bytes {
            findings.extend(engine::run(rules, [(name.as_str(), bytes.as_slice())]));
        }
    }
    let actual = ledger::counts(&findings);
    let over = ledger::excess(&actual, &base);
    let mut stale = ledger::excess(&base, &actual);
    if !selection.is_empty() {
        // A line for a file outside the selection says nothing about this run. A file that is gone is still inside it.
        stale.retain(|(_, path), _| selection.iter().any(|chosen| under(path, chosen)));
    }
    findings.retain(|f| over.contains_key(&(f.rule.to_owned(), f.path.clone())));
    report(&findings, output)?;
    Ok(Scan {
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
