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
/// Returns an error if discovery, file reading, or output fails.
pub fn check(paths: &[PathBuf], output: &mut impl Write) -> io::Result<bool> {
    evaluate(paths, false, output)
}

/// Regenerates the repository ledger within its existing allowances.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn regenerate(output: &mut impl Write) -> io::Result<bool> {
    evaluate(&[], true, output)
}

fn read_ledger(path: &Path) -> io::Result<Option<ledger::Counts>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => ledger::parse(&fs::read(path)?, engine::registry()).map(Some),
        Ok(_) => Err(io::Error::other(".blackhole/ledger must be a regular file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn replace(path: &Path, actual: &ledger::Counts) -> io::Result<()> {
    let contents = ledger::write(actual)?;
    let temp = path.with_file_name(format!("ledger.{}.tmp", std::process::id()));
    let mut file = fs::File::create_new(&temp)?;
    let result = file
        .write_all(contents.as_bytes())
        .and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        fs::remove_file(temp)?;
    }
    result
}

fn evaluate(paths: &[PathBuf], regenerate: bool, output: &mut impl Write) -> io::Result<bool> {
    let (root, names) = files::list(paths)?;
    let directory = root.join(".blackhole");
    let path = directory.join("ledger");
    // Lock the directory because replacing the ledger changes its inode.
    let _lock = if regenerate && directory.try_exists()? {
        let dir = fs::File::open(directory)?;
        dir.lock()?;
        Some(dir)
    } else {
        None
    };
    let baseline = read_ledger(&path)?;
    let rules = engine::registry();
    let mut findings = Vec::new();
    for name in &names {
        let bytes = fs::read(root.join(name))
            .map_err(|e| io::Error::new(e.kind(), format!("{name:?}: {e}")))?;
        findings.extend(engine::run(rules, [(name.as_str(), bytes.as_slice())]));
    }
    let Some(base) = baseline else {
        report(&findings, output)?;
        return Ok(!findings.is_empty());
    };
    let actual = ledger::counts(&findings);
    let selected = (!paths.is_empty()).then(|| names.iter().map(String::as_str).collect());
    let diff = ledger::exact(&base, &actual, selected.as_ref());
    findings.retain(|f| diff.over.contains_key(&(f.rule.to_owned(), f.path.clone())));
    report(&findings, output)?;
    if regenerate {
        if findings.is_empty() && actual != base {
            replace(&path, &actual)?;
        }
        return Ok(!findings.is_empty());
    }
    for (key, allowed) in &diff.stale {
        let (rule, path) = key;
        let found = actual.get(key).copied().unwrap_or(0);
        writeln!(
            output,
            "{path} {rule}: ledger allows {allowed}, found {found}; run blackhole ledger"
        )?;
    }
    Ok(!findings.is_empty() || !diff.stale.is_empty())
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
