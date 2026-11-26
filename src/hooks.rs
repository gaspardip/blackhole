mod patch;

use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Event {
    PreEdit,
    PostEdit,
    Stop,
}

#[must_use]
pub fn run(event: Event) -> ExitCode {
    let mut out = HookOutput(io::stderr().lock());
    match handle(event, &mut out) {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => ExitCode::from(2),
        Err(error) => {
            let _ = writeln!(
                out,
                "blackhole: {}",
                crate::diagnostic_text(&error.to_string())
            );
            if matches!(event, Event::PreEdit) && crate::files::root().is_ok() {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
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

fn handle(event: Event, out: &mut HookOutput<'_>) -> io::Result<bool> {
    const LIMIT: u64 = 64 * 1024 * 1024;

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
    let mut bytes = Vec::new();
    io::stdin().lock().take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other("hook payload exceeds 64 MiB"));
    }
    let envelope: Envelope = serde_json::from_slice(&bytes)?;
    let (field, is_patch): (fn(&Input) -> &Option<String>, _) = match envelope.tool_name.as_str() {
        "Edit" | "Write" | "MultiEdit" => (|p| &p.file_path, false),
        "NotebookEdit" => (|p| &p.notebook_path, false),
        "apply_patch" => (|p| &p.command, true),
        _ => return Ok(false),
    };
    let payload: Payload = serde_json::from_slice(&bytes)?;
    let value = field(&payload.tool_input)
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::other("cannot read edit target"))?;
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
            if path.is_dir() {
                Ok(path)
            } else {
                Err(io::Error::from(io::ErrorKind::NotADirectory))
            }
        })
        .map_err(|e| io::Error::new(e.kind(), format!("invalid hook cwd: {e}")))?;
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
    let identity = if ledger.try_exists()? {
        crate::files::identity(&ledger)
    } else {
        None
    };
    let matches_ledger =
        |path: &Path| identity.is_some_and(|id| crate::files::identity(path) == Some(id));
    let mut seen = None;
    for name in &names {
        let path = cwd.join(name);
        let physical = resolve(&path, &mut seen)?;
        if physical
            .as_ref()
            .is_some_and(|p| p == &ledger || matches_ledger(p))
            || edits.normalized(name).any(|p| matches_ledger(&p))
        {
            return Err(io::Error::other(
                "The ledger only changes through blackhole ledger. Fix code to shrink it, or file an exception with blackhole object.",
            ));
        }
        validate_target(&path)?;
    }
    Ok(false)
}

fn validate_target(path: &Path) -> io::Result<()> {
    if let Err(e) = fs::metadata(path)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(io::Error::new(
            e.kind(),
            format!("Cannot resolve edit target {}: {e}", path.display()),
        ));
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

    fn check(&self, root: &Path, out: &mut HookOutput<'_>) -> io::Result<bool> {
        let policy = crate::Policy::load(root)?;
        let (selected, written) = self.validate(root, policy.paths())?;
        let paths = select(root, selected, &written)?;
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
        Err(e) => Err(io::Error::new(
            e.kind(),
            format!("Cannot resolve edit target: {e}"),
        )),
    }
}

fn select(
    root: &Path,
    mut selected: HashSet<PathBuf>,
    written: &HashSet<PathBuf>,
) -> io::Result<Vec<PathBuf>> {
    if !selected.is_empty() && !written.is_empty() {
        let identities: HashSet<_> = written
            .iter()
            .filter_map(|path| crate::files::identity(path))
            .collect();
        let matches =
            |path: &Path| crate::files::identity(path).is_some_and(|id| identities.contains(&id));
        let hard_links = written
            .iter()
            .any(|path| crate::files::has_hard_links(path));
        let mut directories = vec![root.to_owned()];
        let mut linked = Vec::new();
        while let Some(dir) = directories.pop() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                if entry.file_name() == ".git" {
                    continue;
                }
                let kind = entry.file_type()?;
                let path = entry.path();
                if kind.is_dir() {
                    directories.push(path);
                } else if kind.is_symlink() && path.is_dir() {
                    if hard_links
                        || fs::canonicalize(&path)
                            .is_ok_and(|dir| written.iter().any(|target| target.starts_with(&dir)))
                    {
                        linked.push(path);
                    }
                } else if (kind.is_symlink() || hard_links) && matches(&path) {
                    selected.insert(path);
                }
            }
        }
        let names = crate::files::listed(
            root,
            linked.iter().filter_map(|p| p.strip_prefix(root).ok()),
        )?;
        let mut seen = None;
        for name in names {
            let path = root.join(name);
            if crate::files::resolve(&path, &mut seen).is_ok_and(|real| written.contains(&real))
                || matches(&path)
            {
                selected.insert(path);
            }
        }
    }
    Ok(selected.into_iter().collect())
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
