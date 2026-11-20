pub mod ci;
mod config;
mod coverage;
pub mod engine;
mod files;
mod git;
mod judge_files;
pub mod ledger;
pub mod objections;
mod packs {
    pub(crate) mod ts {
        mod bindings;
        mod config_responses;
        pub(crate) mod dictionary_types;
        mod double_mutations;
        pub(crate) mod function_types;
        mod native_properties;
        pub(crate) mod no_assertionless_tests;
        pub(crate) mod no_chained_type_assertions;
        pub(crate) mod no_comments;
        pub(crate) mod no_explicit_any;
        pub(crate) mod no_explicit_return_types;
        pub(crate) mod no_fixed_sleep;
        pub(crate) mod no_flake_masking;
        pub(crate) mod no_known_value_widening;
        pub(crate) mod no_non_null_assertion;
        pub(crate) mod no_object_parameters;
        pub(crate) mod no_reflect_apply;
        pub(crate) mod no_reflect_get;
        pub(crate) mod no_runtime_typeof;
        pub(crate) mod no_skipped_tests;
        pub(crate) mod no_test_aware_code;
        pub(crate) mod no_test_doubles;
        pub(crate) mod no_unknown_parameters;
        pub(crate) mod no_unknown_returns;
        pub(crate) mod no_unknown_type_aliases;
        pub(crate) mod no_unsafe_dictionary_type;
        pub(crate) mod no_unsignaled_poll;
        pub(crate) mod no_widen_then_assert;
        mod playwright;
        pub(crate) mod reflect_method;
        mod saved_methods;
        pub(crate) mod strict_msw;
        mod test_context;
        mod transports;
        mod values;
    }
    pub(crate) mod core {
        pub(crate) mod no_suppression;
    }
}
pub mod verdict;

use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const MISPLACED: &str = ".blackhole must be a directory";

/// Checks the selected files and returns whether any rule was violated.
///
/// Leaves the process inside the repository's `.blackhole` directory when there is one.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn check(paths: &[PathBuf], output: &mut impl Write) -> io::Result<bool> {
    let scan = scan(&files::root()?, paths, output)?;
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
/// Leaves the process inside the repository's `.blackhole` directory when there is one.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn regenerate(output: &mut impl Write) -> io::Result<bool> {
    let root = files::root()?;
    let _lock = files::lock(&root)?;
    let scan = scan(&root, &[], output)?;
    if !scan.over && !scan.stale.is_empty() {
        let mut shrunk = scan.base;
        shrunk.retain(|key, n| {
            *n = (*n).min(scan.actual.get(key).copied().unwrap_or(0));
            *n > 0
        });
        // A stale row means the ledger was read, so the process is inside the directory.
        replace("ledger", &ledger::write(&shrunk)?)?;
    }
    Ok(scan.over)
}

// The process moves into the directory and reads its files by name, so a link swapped in at its path later cannot move the ledger or the objections outside the repository.
// A link swapped in before the move shows as a working directory that is somewhere else.
fn enter(root: &Path) -> io::Result<bool> {
    let home = root.join(".blackhole");
    match fs::symlink_metadata(&home) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(io::Error::other(MISPLACED)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    }
    match env::set_current_dir(&home) {
        Ok(()) if fs::canonicalize(".")? == home => Ok(true),
        Ok(()) => Err(io::Error::other(MISPLACED)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

// A repository without the directory has neither file, which is what an empty one says.
fn policy(entered: bool, name: &str) -> io::Result<Option<Vec<u8>>> {
    if entered {
        files::policy(Path::new(name))
    } else {
        Ok(None)
    }
}

// The rename is the only write to the file, so a crash leaves the old one whole.
// Both names are relative: the caller has entered the directory.
fn replace(name: &str, contents: &str) -> io::Result<()> {
    let temp = format!("{name}.{}.tmp", std::process::id());
    fs::File::create_new(&temp)?.write_all(contents.as_bytes())?;
    fs::rename(temp, name)
}

struct Scan {
    base: ledger::Counts,
    actual: ledger::Counts,
    over: bool,
    stale: ledger::Counts,
}

fn scan(root: &Path, paths: &[PathBuf], output: &mut impl Write) -> io::Result<Scan> {
    // The selection is resolved before the working directory moves.
    let (mut names, selection) = files::list(root, paths)?;
    let config = config::parse(&files::policy(&root.join("blackhole.toml"))?.unwrap_or_default())?;
    let entered = enter(root)?;
    let base = ledger::load(policy(entered, "ledger")?)?;
    let under = |path: &str, chosen: &Path| root.join(path).starts_with(chosen);
    // A path with no file left is still a selection while the ledger has a row under it. A path that holds the repository would match every file, so it is not one.
    for chosen in &selection {
        let rows = base.keys().map(|(_, path)| path);
        if !(chosen.starts_with(root) && names.iter().chain(rows).any(|path| under(path, chosen))) {
            return Err(io::Error::other(format!(
                "no files match {}",
                chosen.display().to_string().escape_default()
            )));
        }
    }
    names.retain(|name| !config.exclude.is_match(name));
    // Both policy files are read before any file is judged, so a link at either is refused before a scan could follow it.
    let filed = policy(entered, "objections")?.unwrap_or_default();
    let findings = scan_files(names.iter().map(String::as_str), |name| {
        files::read(&root.join(name))
    })?;
    let actual = ledger::counts(&findings);
    let today = objections::Date::today()?;
    let objections = objections::parse(&filed, today)?;
    let over = ledger::excess(&actual, &objections::allowances(&base, &objections, today));
    let mut stale = ledger::excess(&base, &actual);
    if !selection.is_empty() {
        // A line for a file outside the selection says nothing about this run. A file that is gone is still inside it.
        stale.retain(|(_, path), _| selection.iter().any(|chosen| under(path, chosen)));
    }
    report(&findings, &over, output)?;
    Ok(Scan {
        base,
        actual,
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

// Only the findings above their allowance are printed.
fn report(
    findings: &[engine::Finding],
    over: &ledger::Counts,
    output: &mut impl Write,
) -> io::Result<()> {
    let findings: Vec<_> = findings
        .iter()
        .filter(|f| over.contains_key(&(f.rule.to_owned(), f.path.clone())))
        .collect();
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
///
/// Leaves the process inside the repository's `.blackhole` directory.
///
/// # Errors
/// Rejects invalid objections, absent overages and repeated renewals.
pub fn object(rule: String, path: String, reason: Option<&str>, renew: bool) -> io::Result<()> {
    let root = files::root()?;
    let _lock = files::lock(&root)?;
    let entered = enter(&root)?;
    let bytes = policy(entered, "objections")?.unwrap_or_default();
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
        let renewed = objections::Objection {
            renewal: Some(today),
            ..row.clone()
        }
        .line(&key);
        let prefix = format!("{}\t{}\t", key.0, key.1);
        contents = contents
            .split_inclusive('\n')
            .map(|line| {
                if line.starts_with(&prefix) {
                    &renewed
                } else {
                    line
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
        let scan = scan(&root, &[], &mut io::sink())?;
        let extra = scan
            .actual
            .get(&key)
            .copied()
            .unwrap_or(0)
            .saturating_sub(scan.base.get(&key).copied().unwrap_or(0));
        if extra == 0 || ledger::invalid_path(&key.1) {
            return Err(io::Error::other("no representable overage"));
        }
        let filed = objections::Objection {
            extra,
            filed: today,
            renewal: None,
            reason: reason.to_owned(),
        };
        contents.push_str(&filed.line(&key));
    }
    if !entered {
        // Creating the directory never follows a link at its path, and entering it checks where the process landed.
        fs::create_dir(root.join(".blackhole"))?;
        if !enter(&root)? {
            return Err(io::Error::other(MISPLACED));
        }
    }
    replace("objections", &contents)
}
