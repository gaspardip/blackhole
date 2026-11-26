pub mod ci;
pub mod config;
mod coverage;
pub mod engine;
mod files;
mod git;
pub mod hooks;
pub mod init;
mod judge_files;
pub mod ledger;
pub mod objections;
mod packs {
    pub(crate) mod boundary;
    pub(crate) mod ts {
        mod array_methods;
        mod bindings;
        mod config_responses;
        pub(crate) mod dictionary_types;
        mod double_mutations;
        pub(crate) mod function_types;
        mod local_alias;
        mod native_properties;
        pub(crate) mod no_assertionless_tests;
        pub(crate) mod no_chained_type_assertions;
        pub(crate) mod no_comments;
        pub(crate) mod no_conditional_empty_object_spread;
        pub(crate) mod no_explicit_any;
        pub(crate) mod no_explicit_return_types;
        pub(crate) mod no_fixed_sleep;
        pub(crate) mod no_flake_masking;
        pub(crate) mod no_known_value_widening;
        pub(crate) mod no_non_null_assertion;
        pub(crate) mod no_object_parameters;
        pub(crate) mod no_reduce_accumulator_copy;
        pub(crate) mod no_reflect_apply;
        pub(crate) mod no_reflect_get;
        pub(crate) mod no_runtime_typeof;
        pub(crate) mod no_shape_in_symbol_names;
        pub(crate) mod no_skipped_tests;
        pub(crate) mod no_test_aware_code;
        pub(crate) mod no_test_doubles;
        pub(crate) mod no_test_removal;
        pub(crate) mod no_unknown_parameters;
        pub(crate) mod no_unknown_returns;
        pub(crate) mod no_unknown_type_aliases;
        pub(crate) mod no_unsafe_dictionary_type;
        pub(crate) mod no_unsignaled_poll;
        pub(crate) mod no_widen_then_assert;
        mod playwright;
        pub(crate) mod reflect_method;
        mod regions;
        mod saved_methods;
        pub(crate) mod strict_msw;
        pub(crate) mod test_context;
        mod transports;
        pub(crate) mod values;
    }
    pub(crate) mod core {
        pub(crate) mod no_suppression;
    }
}
pub mod verdict;

#[cfg(test)]
mod advisory_tests;

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
    check_paths(&files::root()?, Targets::Command(paths), output)
}

fn check_paths(root: &Path, targets: Targets<'_>, output: &mut impl Write) -> io::Result<bool> {
    let scan = scan(root, targets, output)?;
    report_stale(&scan.stale, &scan.actual, targets.hook(), output)?;
    Ok(scan.over || !scan.stale.is_empty())
}

fn report_stale(
    stale: &ledger::Counts,
    actual: &ledger::Counts,
    hook: bool,
    output: &mut impl Write,
) -> io::Result<()> {
    for (key, allowed) in stale {
        let (rule, path) = key;
        let path = rendered(path, hook);
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
    let scan = scan(&root, Targets::Command(&[]), output)?;
    if !scan.over
        && let Some(bytes) = scan.ledger_bytes
    {
        let mut shrunk = scan.base;
        shrunk.retain(|key, n| {
            *n = (*n).min(scan.actual.get(key).copied().unwrap_or(0));
            *n > 0
        });
        let contents = ledger::write(&shrunk)?;
        if contents.as_bytes() != bytes {
            replace("ledger", &contents)?;
        }
    }
    Ok(scan.over)
}

// The process moves into the directory and reads its files by name, so a link swapped in at its path later cannot move the ledger or the objections outside the repository.
// A link swapped in before the move shows as a working directory that is somewhere else.
fn enter(root: &Path, directory: &str, create: bool) -> io::Result<bool> {
    let home = root.join(directory);
    let misplaced = || io::Error::other(format!("{directory} must be a directory"));
    if create
        && let Err(e) = fs::create_dir(&home)
        && e.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(e);
    }
    match fs::symlink_metadata(&home) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(misplaced()),
        Err(e) if !create && e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    }
    match env::set_current_dir(&home) {
        Ok(()) if fs::canonicalize(".")? == home => Ok(true),
        Ok(()) => Err(misplaced()),
        Err(e) if !create && e.kind() == io::ErrorKind::NotFound => Ok(false),
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
fn replace(name: &str, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let temp = format!("{name}.{}.tmp", std::process::id());
    let permissions = files::regular(Path::new(name))?.map(|m| m.permissions());
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Restrict the temporary file from its first open; after writing, restore bits masked by umask or cleared by the write.
    #[cfg(unix)]
    if let Some(mode) = &permissions {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(mode.mode());
    }
    let file = options.open(&temp)?;
    fill(file, contents.as_ref(), permissions)
        .and_then(|()| fs::rename(&temp, name))
        .map_err(|error| match fs::remove_file(&temp) {
            Ok(()) => error,
            Err(cleanup) => {
                io::Error::new(error.kind(), format!("{error}; removing {temp}: {cleanup}"))
            }
        })
}

// Takes the file so it is closed before the rename or the removal.
fn fill(
    mut file: fs::File,
    contents: &[u8],
    permissions: Option<fs::Permissions>,
) -> io::Result<()> {
    file.write_all(contents)?;
    permissions.map_or(Ok(()), |mode| file.set_permissions(mode))
}

#[derive(Default)]
struct Scan {
    ledger_bytes: Option<Vec<u8>>,
    base: ledger::Counts,
    actual: ledger::Counts,
    over: bool,
    stale: ledger::Counts,
}

#[derive(Clone, Copy)]
enum Targets<'a> {
    Command(&'a [PathBuf]),
    HookStop,
    Edits(Option<&'a hooks::EditNames<'a>>),
}

impl Targets<'_> {
    fn hook(self) -> bool {
        !matches!(self, Self::Command(_))
    }
}

fn scan(root: &Path, targets: Targets<'_>, output: &mut impl Write) -> io::Result<Scan> {
    // The selection is resolved before the working directory moves.
    let (names, selection) = match targets {
        Targets::Command(paths) => files::list(root, paths)?,
        Targets::HookStop | Targets::Edits(Some(_)) => files::list(root, &[])?,
        Targets::Edits(None) => (Vec::new(), Vec::new()),
    };
    let config = config::parse(&files::policy(&root.join("blackhole.toml"))?.unwrap_or_default())?;
    let entered = enter(root, ".blackhole", false)?;
    let ledger_bytes = policy(entered, "ledger")?;
    let base = ledger::load(ledger_bytes.as_deref())?;
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
    let edits = match targets {
        Targets::Edits(None) => return Ok(Scan::default()),
        Targets::Edits(Some(edits)) => {
            let selected = edits.select(root, &names, &base)?;
            if selected.is_empty() {
                return Ok(Scan::default());
            }
            Some(selected)
        }
        Targets::Command(_) | Targets::HookStop => None,
    };
    let selected = |name: &str| {
        edits.as_ref().map_or_else(
            || selection.is_empty() || selection.iter().any(|chosen| under(name, chosen)),
            |edits| edits.contains(Path::new(name)),
        )
    };
    // Both policy files are read before any file is judged.
    let filed = policy(entered, "objections")?.unwrap_or_default();
    let names = names
        .iter()
        .map(String::as_str)
        .filter(|name| selected(name));
    let findings = scan_files(&config, Vec::new(), names, targets.hook(), |name| {
        files::read(&root.join(name))
    })?;
    let actual = ledger::counts(&findings);
    let today = objections::Date::today()?;
    let objections = objections::parse(&filed, today)?;
    let over = ledger::excess(&actual, &objections::allowances(&base, &objections, today));
    let mut stale = ledger::excess(&base, &actual);
    stale.retain(|(_, path), _| selected(path));
    report(&findings, &over, targets.hook(), output)?;
    Ok(Scan {
        ledger_bytes,
        base,
        actual,
        over: !over.is_empty(),
        stale,
    })
}

fn scan_files<'a>(
    config: &config::Config,
    mut findings: Vec<engine::Finding>,
    names: impl Iterator<Item = &'a str>,
    hook: bool,
    mut read: impl FnMut(&str) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<Vec<engine::Finding>> {
    for name in names.filter(|name| !config.exclude.is_match(name)) {
        let bytes = read(name).map_err(|e| {
            let path = if hook {
                name.into()
            } else {
                format!("{name:?}")
            };
            io::Error::new(e.kind(), format!("{path}: {e}"))
        })?;
        if let Some(bytes) = bytes {
            findings.extend(engine::run_configured(
                engine::registry(),
                config.engine_settings(),
                [(name, bytes.as_slice())],
            ));
        }
    }
    engine::sort_findings(&mut findings);
    Ok(findings)
}

fn rendered(text: &str, hook: bool) -> std::borrow::Cow<'_, str> {
    if hook {
        hooks::hook_text(text)
    } else {
        text.into()
    }
}

fn report(
    findings: &[engine::Finding],
    over: &ledger::Counts,
    hook: bool,
    output: &mut impl Write,
) -> io::Result<()> {
    let findings = findings.iter().filter(|f| {
        // Advisory findings need output even when there is no overage.
        f.gate == engine::Gate::Advisory || over.contains_key(&(f.rule.to_owned(), f.path.clone()))
    });
    print_findings(findings, hook, output)
}

fn print_findings<'a>(
    findings: impl IntoIterator<Item = &'a engine::Finding>,
    hook: bool,
    output: &mut impl Write,
) -> io::Result<()> {
    let mut matched = std::collections::BTreeSet::new();
    for f in findings {
        matched.insert(f.rule);
        write!(
            output,
            "{}:{}:{} {}",
            rendered(&f.path, hook),
            f.line,
            f.col,
            f.rule
        )?;
        if f.gate == engine::Gate::Advisory {
            write!(output, " [advisory]")?;
        }
        if let Some(note) = &f.note {
            write!(output, " {}", rendered(note, hook))?;
        }
        writeln!(output)?;
    }
    for rule in engine::registry() {
        if matched.contains(rule.id) {
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
    let entered = enter(&root, ".blackhole", false)?;
    let bytes = policy(entered, "objections")?.unwrap_or_default();
    let today = objections::Date::today()?;
    let rows = objections::parse(&bytes, today)?;
    // Old rows for a rule that became advisory still parse, but a new one would allow nothing.
    let advisory = engine::registry()
        .iter()
        .any(|r| r.id == rule && r.gate == engine::Gate::Advisory);
    if advisory || !engine::can_allow(engine::registry(), &rule) {
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
        let scan = scan(&root, Targets::Command(&[]), &mut io::sink())?;
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
        if !enter(&root, ".blackhole", false)? {
            return Err(io::Error::other(MISPLACED));
        }
    }
    replace("objections", &contents)
}
