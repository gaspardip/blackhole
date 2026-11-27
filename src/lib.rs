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
    env, fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

/// What a failed command leaves to its caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The caller typed something invalid.
    Usage,
    /// The checkout's own content cannot be judged, which is the author's to fix.
    Red,
    /// Something outside the checkout's control kept the tool from judging.
    NoVerdict,
}

#[derive(Debug)]
struct Classified {
    fault: Fault,
    message: String,
}

impl fmt::Display for Classified {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Classified {}

pub(crate) fn usage(message: impl fmt::Display) -> io::Error {
    classified(Fault::Usage, message)
}

pub(crate) fn red(message: impl fmt::Display) -> io::Error {
    classified(Fault::Red, message)
}

/// Marks a failure outside the checkout's control.
#[must_use]
pub fn no_verdict(message: impl fmt::Display) -> io::Error {
    classified(Fault::NoVerdict, message)
}

// Classified by the cause, so a resource failure while reading stays no verdict.
pub(crate) fn unreadable(name: &str, error: &io::Error) -> io::Error {
    classified(
        fault(error),
        format!(
            "cannot read {}: {error}; make it readable or remove it",
            diagnostic_text(name)
        ),
    )
}

fn classified(fault: Fault, message: impl fmt::Display) -> io::Error {
    io::Error::other(Classified {
        fault,
        message: message.to_string(),
    })
}

/// The fault an error was marked with where it was created.
pub(crate) fn mark(error: &io::Error) -> Option<Fault> {
    let classified = error.get_ref()?.downcast_ref::<Classified>()?;
    Some(classified.fault)
}

/// Classifies an error from a command that judges the checkout.
///
/// The checkout's content can cause any unmarked failure, so it is red unless the system ran out of a resource.
#[must_use]
pub fn fault(error: &io::Error) -> Fault {
    use io::ErrorKind::{
        Interrupted, OutOfMemory, QuotaExceeded, ResourceBusy, StorageFull, TimedOut, WouldBlock,
    };
    mark(error).unwrap_or_else(|| {
        if matches!(
            error.kind(),
            Interrupted
                | OutOfMemory
                | QuotaExceeded
                | ResourceBusy
                | StorageFull
                | TimedOut
                | WouldBlock
        ) {
            Fault::NoVerdict
        } else {
            Fault::Red
        }
    })
}

/// Checks the selected files and returns whether any rule was violated.
///
/// Leaves the process inside the repository's `.blackhole` directory when there is one.
///
/// # Errors
/// Returns an error if discovery, ledger access, file reading, or output fails.
pub fn check(paths: &[PathBuf], output: &mut impl Write) -> io::Result<bool> {
    let cwd = env::current_dir()?;
    let root = files::root()?;
    let policy = Policy::load(&root)?;
    let listing = files::list(&root, &cwd, paths, policy.paths())?;
    if let Some(path) = listing.unmatched {
        return Err(usage(format!(
            "no files match {}",
            diagnostic_text(&path.to_string_lossy())
        )));
    }
    check_paths(&root, &policy, &listing, output)
}

fn check_paths(
    root: &Path,
    policy: &Policy,
    listing: &files::Listing<'_>,
    output: &mut impl Write,
) -> io::Result<bool> {
    let scan = scan(root, policy, listing, output)?;
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
        let path = diagnostic_text(path);
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
    let policy = Policy::load(&root)?;
    let scan = scan_all(&root, &policy, output)?;
    if !scan.over
        && let Some(bytes) = policy.ledger_bytes
    {
        let mut shrunk = policy.base;
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
    let misplaced = || red(format!("{directory} must be a directory"));
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

struct Policy {
    config: config::Config,
    entered: bool,
    ledger_bytes: Option<Vec<u8>>,
    base: ledger::Counts,
}

impl Policy {
    fn load(root: &Path) -> io::Result<Self> {
        let config =
            config::parse(&files::policy(&root.join("blackhole.toml"))?.unwrap_or_default())
                .map_err(red)?;
        let entered = enter(root, ".blackhole", false)?;
        let ledger_bytes = policy(entered, "ledger")?;
        let base = ledger::load(ledger_bytes.as_deref()).map_err(red)?;
        Ok(Self {
            config,
            entered,
            ledger_bytes,
            base,
        })
    }

    fn paths(&self) -> impl Iterator<Item = &str> {
        self.base.keys().map(|(_, path)| path.as_str())
    }
}

struct Scan {
    actual: ledger::Counts,
    over: bool,
    stale: ledger::Counts,
}

fn scan_all(root: &Path, policy: &Policy, output: &mut impl Write) -> io::Result<Scan> {
    scan(
        root,
        policy,
        &files::list(root, root, &[], policy.paths())?,
        output,
    )
}

fn scan(
    root: &Path,
    rules: &Policy,
    listing: &files::Listing<'_>,
    output: &mut impl Write,
) -> io::Result<Scan> {
    // Both policy files are read before any file is judged.
    let filed = policy(rules.entered, "objections")?.unwrap_or_default();
    let names = listing.names.iter().map(String::as_str);
    let findings = scan_files(
        &rules.config,
        Vec::new(),
        names,
        (),
        |()| inventory(root, listing),
        |(), name| files::read(&root.join(name)).map_err(|e| unreadable(name, &e)),
    )?;
    let actual = ledger::counts(&findings);
    let today = objections::Date::today()?;
    let objections = objections::parse(&filed, today).map_err(red)?;
    let over = ledger::excess(
        &actual,
        &objections::allowances(&rules.base, &objections, today),
    );
    let mut stale = ledger::excess(&rules.base, &actual);
    if let Some(rows) = &listing.rows {
        stale.retain(|(_, path), _| rows.contains(path.as_str()));
    }
    report(&findings, &over, output)?;
    Ok(Scan {
        actual,
        over: !over.is_empty(),
        stale,
    })
}

// A listing narrowed to selected paths still resolves imports against every file in the repository.
fn inventory(
    root: &Path,
    listing: &files::Listing<'_>,
) -> io::Result<packs::boundary::RepositoryPaths> {
    if listing.rows.is_none() {
        return files::repository_paths(root, listing.names.iter().map(String::as_str));
    }
    let all = files::list(root, root, &[], std::iter::empty())?;
    files::repository_paths(root, all.names.iter().map(String::as_str))
}

fn scan_files<'a, S: Send>(
    config: &config::Config,
    mut findings: Vec<engine::Finding>,
    names: impl Iterator<Item = &'a str>,
    mut source: S,
    inventory: impl FnOnce(&mut S) -> io::Result<packs::boundary::RepositoryPaths>,
    mut read: impl FnMut(&mut S, &str) -> io::Result<Option<Vec<u8>>> + Send,
) -> io::Result<Vec<engine::Finding>> {
    engine::scan(
        engine::registry(),
        config.engine_settings(),
        config.banned_imports(),
        |enabled| {
            let paths = enabled.then(|| inventory(&mut source).map_err(|error| error.to_string()));
            let reader = packs::boundary::Reader::new(move |name| read(&mut source, name));
            (paths, reader, ())
        },
        |scanner, reader, ()| -> io::Result<()> {
            for name in names.filter(|name| !config.exclude.is_match(name)) {
                if let Some(bytes) = reader.read(name)? {
                    scanner.file(name, &bytes, &mut findings);
                }
            }
            Ok(())
        },
    )?;
    engine::sort_findings(&mut findings);
    Ok(findings)
}

fn report(
    findings: &[engine::Finding],
    over: &ledger::Counts,
    output: &mut impl Write,
) -> io::Result<()> {
    let findings = findings.iter().filter(|f| {
        // Advisory findings need output even when there is no overage.
        f.gate == engine::Gate::Advisory || over.contains_key(&(f.rule.to_owned(), f.path.clone()))
    });
    print_findings(findings, output)
}

fn print_findings<'a>(
    findings: impl IntoIterator<Item = &'a engine::Finding>,
    output: &mut impl Write,
) -> io::Result<()> {
    let mut matched = std::collections::BTreeSet::new();
    for f in findings {
        matched.insert(f.rule);
        write!(
            output,
            "{}:{}:{} {}",
            diagnostic_text(&f.path),
            f.line,
            f.col,
            f.rule
        )?;
        if f.gate == engine::Gate::Advisory {
            write!(output, " [advisory]")?;
        }
        if let Some(note) = &f.note {
            write!(output, " {}", diagnostic_text(note))?;
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
    let rows = objections::parse(&bytes, today).map_err(red)?;
    // Old rows for a rule that became advisory still parse, but a new one would allow nothing.
    let advisory = engine::registry()
        .iter()
        .any(|r| r.id == rule && r.gate == engine::Gate::Advisory);
    if advisory || !engine::can_allow(engine::registry(), &rule) {
        return Err(usage("rule cannot be objected to"));
    }
    let key = (rule, path);
    let mut contents = String::from_utf8(bytes).map_err(red)?;
    if renew {
        if reason.is_some() {
            return Err(usage("renewal takes no reason"));
        }
        let row = rows.get(&key).ok_or_else(|| usage("no objection"))?;
        if row.renewal.is_some() {
            return Err(usage("already renewed once"));
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
            return Err(usage("duplicate objection"));
        }
        let reason = reason
            .filter(|s| !s.trim().is_empty() && !s.contains(['\r', '\n', '\t']))
            .ok_or_else(|| usage("invalid reason"))?;
        let policy = Policy::load(&root)?;
        let scan = scan_all(&root, &policy, &mut io::sink())?;
        let extra = scan
            .actual
            .get(&key)
            .copied()
            .unwrap_or(0)
            .saturating_sub(policy.base.get(&key).copied().unwrap_or(0));
        if extra == 0 || ledger::invalid_path(&key.1) {
            return Err(usage("no representable overage"));
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
        enter(&root, ".blackhole", true)?;
    }
    replace("objections", &contents)
}

/// Escapes characters that could disrupt a line of CLI output.
/// Escapes the characters that would break a diagnostic line.
#[must_use]
pub fn diagnostic_text(text: &str) -> std::borrow::Cow<'_, str> {
    let control = |c: char| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}');
    // A literal escape must remain distinguishable from the control character it spells.
    if !text.chars().any(control)
        && !text.match_indices('\\').any(|(index, _)| {
            let rest = &text[index + 1..];
            rest.starts_with(['0', 'n', 'r', 't']) || rest.starts_with("u{")
        })
    {
        return text.into();
    }
    let mut result = String::new();
    for c in text.chars() {
        if control(c) || c == '\\' {
            result.extend(c.escape_debug());
        } else {
            result.push(c);
        }
    }
    result.into()
}
