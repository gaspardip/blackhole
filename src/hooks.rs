mod patch;

use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Event {
    PreEdit,
    PostEdit,
    Stop,
}

const LIMIT: u64 = 64 * 1024 * 1024;

#[must_use]
pub fn run(event: Event) -> ExitCode {
    // The agent writes the payload before it waits, so it is drained first; Stop needs it only for the loop guard.
    let payload = match event {
        // Run by hand from a terminal, Stop has no host payload to wait for.
        Event::Stop if io::stdin().is_terminal() => Ok(Vec::new()),
        _ => match payload() {
            Err(_) if matches!(event, Event::Stop) => Ok(Vec::new()),
            read => read,
        },
    };
    crate::deadline::activity(|| "judging the hook".into());
    // Reading the policy moves the process into `.blackhole`, and a retry must resolve the payload from where the agent started.
    let start = std::env::current_dir();
    let (bytes, (text, result)) = match payload {
        Ok(bytes) => {
            let first = attempt(event, &bytes);
            // A failure that is no verdict can be transient, so it gets a second judgment.
            let judged = if first
                .1
                .as_ref()
                .is_err_and(|e| crate::fault(e) == crate::Fault::NoVerdict)
            {
                match start.as_ref().map(std::env::set_current_dir) {
                    Ok(Err(e)) => (first.0, Err(e)),
                    _ => attempt(event, &bytes),
                }
            } else {
                first
            };
            (bytes, judged)
        }
        Err(error) => (Vec::new(), (Vec::new(), Err(error))),
    };
    let mut out = HookOutput(io::stderr().lock());
    let _ = out.write_all(&text);
    let error = match result {
        Ok(false) => return ExitCode::SUCCESS,
        Ok(true) => return ExitCode::from(2),
        Err(error) => error,
    };
    let cause = crate::diagnostic_text(&error.to_string()).into_owned();
    let fault = crate::fault(&error);
    if fault != crate::Fault::NoVerdict {
        let _ = writeln!(out, "blackhole: {cause}");
    } else if matches!(event, Event::Stop) && continued(&bytes) {
        // Both hosts drop stderr when a hook exits 0 but show a systemMessage from stdout.
        let message = format!(
            "blackhole: no verdict: {cause}; the judgment could not run, so the agent may stop"
        );
        let _ = writeln!(
            io::stdout(),
            "{}",
            serde_json::json!({ "systemMessage": message })
        );
        return ExitCode::SUCCESS;
    } else {
        let _ = writeln!(out, "blackhole: no verdict: {cause}");
    }
    if crate::files::outside_checkout() {
        return ExitCode::FAILURE;
    }
    match (fault, event) {
        // The edit already happened, and a payload the hook cannot read leaves nothing to report on.
        (crate::Fault::Usage, Event::PostEdit) => ExitCode::FAILURE,
        // A guard that cannot decide inside a checkout must block, since Codex ignores exit 1.
        _ => ExitCode::from(2),
    }
}

fn attempt(event: Event, bytes: &[u8]) -> (Vec<u8>, io::Result<bool>) {
    let mut text = Vec::new();
    let result = handle(event, bytes, &mut text);
    (text, result)
}

fn payload() -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    io::stdin().lock().take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(crate::usage("hook payload exceeds 64 MiB"));
    }
    Ok(bytes)
}

// Claude Code and Codex set `stop_hook_active` once a stop hook has continued the turn; an unreadable payload keeps the block.
fn continued(bytes: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct Stop {
        #[serde(default)]
        stop_hook_active: bool,
    }
    serde_json::from_slice::<Stop>(bytes).is_ok_and(|stop| stop.stop_hook_active)
}

struct HookOutput<'a>(io::StderrLock<'a>);

impl Write for HookOutput<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // The agent still needs the hook verdict when its diagnostic pipe has closed.
        let _ = self.0.write_all(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn handle(event: Event, bytes: &[u8], out: &mut Vec<u8>) -> io::Result<bool> {
    if matches!(event, Event::Stop) {
        let red = crate::check(&[], out)?;
        if red {
            writeln!(
                out,
                "Fix the findings or file an objection with blackhole object for a person to approve."
            )?;
        }
        return Ok(red);
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(crate::usage)?;
    let (field, is_patch): (fn(&Input) -> &Option<String>, _) = match envelope.tool_name.as_str() {
        "Edit" | "Write" | "MultiEdit" => (|p| &p.file_path, false),
        "NotebookEdit" => (|p| &p.notebook_path, false),
        "apply_patch" => (|p| &p.command, true),
        _ => return Ok(false),
    };
    let payload: Payload = serde_json::from_slice(bytes).map_err(crate::usage)?;
    let value = field(&payload.tool_input)
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| crate::usage("cannot read edit target"))?;
    let mut names = if is_patch {
        patch::targets(value)?
    } else {
        vec![value]
    };
    if names.is_empty() {
        return Ok(false);
    }
    let supplied = Path::new(payload.cwd.as_deref().unwrap_or("."));
    let spelling = std::env::current_dir()?.join(supplied);
    let cwd = fs::canonicalize(supplied)
        .and_then(|path| {
            if fs::metadata(&path)?.is_dir() {
                Ok(path)
            } else {
                Err(io::Error::from(io::ErrorKind::NotADirectory))
            }
        })
        .map_err(|e| crate::unresolvable(&format!("hook cwd {}", supplied.display()), &e))?;
    names.sort_unstable();
    names.dedup();
    let root = crate::files::root()?;
    let edits = EditNames {
        cwd: &cwd,
        spelling: &spelling,
        names: &names,
    };
    if matches!(event, Event::PostEdit) {
        return edits.check(&root, out);
    }
    let ledger = root.join(crate::files::LEDGER_PATH);
    let mut seen = None;
    let protected = resolve(&ledger, &mut seen)?;
    let identity = if ledger.try_exists()? {
        crate::files::identity(&ledger)
    } else {
        None
    };
    let same_file =
        |path: &Path| identity.is_some_and(|id| crate::files::identity(path) == Some(id));
    // Case-insensitive filesystems reach an absent ledger through any case variant, and no inode exists yet to compare.
    let fold = |p: &Path| p.to_string_lossy().to_lowercase();
    let folded = [Some(fold(&ledger)), protected.as_deref().map(fold)];
    let matches_protected = |p: &Path| folded.contains(&Some(fold(p)));
    let mut alternate = None;
    for name in &names {
        let path = cwd.join(name);
        let physical = edit_destination(&path, &mut seen)?;
        if physical
            .as_deref()
            .is_some_and(|p| matches_protected(p) || same_file(p))
            || edits.normalized(name).any(|p| {
                same_file(&p)
                    || edit_destination(&p, &mut alternate)
                        .is_ok_and(|real| real.as_deref().is_some_and(matches_protected))
            })
        {
            return Err(crate::red(
                "The ledger only changes through blackhole ledger. Fix code to shrink it, or file an exception with blackhole object.",
            ));
        }
        validate_target(&path)?;
    }
    Ok(false)
}

// The writer follows a final link even when its target does not exist yet.
fn edit_destination(path: &Path, seen: &mut crate::files::Seen) -> io::Result<Option<PathBuf>> {
    let mut resolved = resolve(path, seen)?;
    for _ in 0..40 {
        let Some(path) = resolved else {
            return Ok(None);
        };
        match fs::read_link(&path) {
            Ok(target) => resolved = resolve(&path.with_file_name(target), seen)?,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::InvalidInput
                        | io::ErrorKind::NotFound
                        | io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(Some(path));
            }
            Err(e) => {
                return Err(unresolved(&path, &e));
            }
        }
    }
    Err(unresolved(
        path,
        &io::Error::other("too many levels of symbolic links"),
    ))
}

fn unresolved(path: &Path, e: &io::Error) -> io::Error {
    crate::unresolvable(&format!("edit target {}", path.display()), e)
}

fn validate_target(path: &Path) -> io::Result<()> {
    if let Err(e) = fs::metadata(path)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(unresolved(path, &e));
    }
    Ok(())
}

#[derive(Deserialize)]
struct Envelope {
    tool_name: String,
}

#[derive(Deserialize)]
struct Payload {
    cwd: Option<String>,
    #[serde(default)]
    tool_input: Input,
}

#[derive(Default, Deserialize)]
struct Input {
    file_path: Option<String>,
    notebook_path: Option<String>,
    command: Option<String>,
}

fn lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        if part == std::path::Component::ParentDir {
            result.pop();
        } else {
            result.push(part);
        }
    }
    result
}

struct EditNames<'a> {
    cwd: &'a Path,
    spelling: &'a Path,
    names: &'a [&'a str],
}

impl EditNames<'_> {
    // The name as a writer that drops `..` textually reads it, once per spelling of the directory it starts from.
    fn normalized(&self, name: &str) -> impl Iterator<Item = PathBuf> {
        [
            Some(self.cwd),
            (self.spelling != self.cwd).then_some(self.spelling),
        ]
        .into_iter()
        .flatten()
        .map(move |base| lexical(&base.join(name)))
    }

    fn check(&self, root: &Path, out: &mut Vec<u8>) -> io::Result<bool> {
        let policy = crate::Policy::load(root)?;
        let (mut selected, written) = self.validate(root, policy.paths())?;
        if !selected.is_empty() && !written.is_empty() {
            select_aliases(root, &mut selected, &written, policy.paths())?;
        }
        let paths: Vec<_> = selected.into_iter().collect();
        if paths.is_empty() {
            return Ok(false);
        }
        let listed = crate::files::list(root, self.cwd, &paths, policy.paths())?;
        if listed.names.is_empty() && listed.rows.as_ref().is_some_and(HashSet::is_empty) {
            return Ok(false);
        }
        crate::check_paths(root, &policy, &listed, out)
    }

    fn validate<'a>(
        &self,
        root: &Path,
        rows: impl Iterator<Item = &'a str>,
    ) -> io::Result<(HashSet<PathBuf>, HashSet<PathBuf>)> {
        let mut seen = None;
        let mut stored: HashMap<PathBuf, Vec<&str>> = HashMap::new();
        for name in rows {
            let path = root.join(name);
            if let Ok(real) = crate::files::resolve(&path, &mut seen)
                && real != path
            {
                stored.entry(real).or_default().push(name);
            }
            stored.entry(path).or_default().push(name);
        }
        let mut selected = HashSet::new();
        let mut written = HashSet::new();
        self.visit(|path| {
            if let Some(rows) = stored.get(path) {
                selected.extend(rows.iter().map(|name| root.join(name)));
            }
            if path.starts_with(root)
                && path.symlink_metadata().is_ok_and(|m| !m.is_dir())
                && !path.is_dir()
            {
                selected.insert(path.to_owned());
            }
            if let Ok(real) = fs::canonicalize(path)
                && real.is_file()
            {
                if real.starts_with(root) {
                    selected.insert(real.clone());
                }
                written.insert(real);
            }
        })?;
        Ok((selected, written))
    }

    fn visit(&self, mut visit: impl FnMut(&Path)) -> io::Result<()> {
        let mut physical = None;
        let mut followed = [None, None];
        for name in self.names {
            let path = self.cwd.join(name);
            validate_target(&path)?;
            let resolved = resolve(&path, &mut physical)?;
            for (normalized, seen) in self.normalized(name).zip(&mut followed) {
                if normalized != path
                    && let Ok(Some(real)) = resolve(&normalized, seen)
                    && Some(&real) != resolved.as_ref()
                {
                    visit(&real);
                }
                if Some(&normalized) != resolved.as_ref() {
                    visit(&normalized);
                }
            }
            if let Some(resolved) = resolved {
                visit(&resolved);
            }
        }
        Ok(())
    }
}

fn resolve(path: &Path, seen: &mut crate::files::Seen) -> io::Result<Option<PathBuf>> {
    match crate::files::resolve(path, seen) {
        Ok(p) => Ok(Some(p)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(unresolved(path, &e)),
    }
}

fn select_aliases<'a>(
    root: &Path,
    selected: &mut HashSet<PathBuf>,
    written: &HashSet<PathBuf>,
    rows: impl Iterator<Item = &'a str>,
) -> io::Result<()> {
    let identities: HashSet<_> = written
        .iter()
        .filter_map(|path| crate::files::identity(path))
        .collect();
    let mut seen = None;
    let mut matches = |name: &Path| {
        let path = root.join(name);
        crate::files::resolve(&path, &mut seen).is_ok_and(|real| written.contains(&real))
            || crate::files::identity(&path).is_some_and(|id| identities.contains(&id))
    };
    selected.extend(
        rows.map(Path::new)
            .filter(|name| matches(name))
            .map(|name| root.join(name)),
    );
    let names = crate::files::listed_matching(root, [Path::new(".")], matches)?;
    selected.extend(names.into_iter().map(|name| root.join(name)));
    Ok(())
}

#[cfg(test)]
#[test]
fn single_target_patch_hunks() {
    for hunk in [
        "*** Add File: a.py",
        "*** Delete File: a.py",
        "*** Update File: a.py\n+x",
    ] {
        let input = format!("*** Begin Patch\n{hunk}\n*** End Patch");
        assert_eq!(patch::targets(&input).unwrap(), ["a.py"]);
    }
}
