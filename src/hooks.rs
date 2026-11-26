mod patch;

use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
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
        let red = crate::check_paths(&crate::files::root()?, crate::Targets::HookStop, out)?;
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
    let tool = match envelope.tool_name.as_str() {
        "Edit" | "Write" | "MultiEdit" => Tool::File,
        "NotebookEdit" => Tool::NotebookEdit,
        "apply_patch" => Tool::Patch,
        _ => return Ok(false),
    };
    let payload: Payload = serde_json::from_slice(&bytes)?;
    let mut names = payload.targets(tool)?;
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
        let listed = edits.validate(&root)?.then_some(&edits);
        return crate::check_paths(&root, crate::Targets::Edits(listed), out);
    }
    let ledger = root.join(crate::files::LEDGER_PATH);
    let metadata = match fs::metadata(&ledger) {
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let mut seen = None;
    for name in &names {
        let path = cwd.join(name);
        let physical = resolve(&path, &mut seen)?;
        if physical
            .as_ref()
            .is_some_and(|p| same_file(p, &ledger, metadata.as_ref()))
            || (metadata.is_some()
                && edits.normalized(name).any(|normalized| {
                    physical.as_ref() != Some(&normalized)
                        && same_file(&normalized, &ledger, metadata.as_ref())
                }))
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

#[derive(Clone, Copy)]
enum Tool {
    File,
    NotebookEdit,
    Patch,
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

impl Payload {
    fn targets(&self, tool: Tool) -> io::Result<Vec<&str>> {
        let field = match tool {
            Tool::File => &self.tool_input.file_path,
            Tool::NotebookEdit => &self.tool_input.notebook_path,
            Tool::Patch => &self.tool_input.command,
        };
        let value = field
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| io::Error::other("cannot read edit target"))?;
        if matches!(tool, Tool::Patch) {
            patch::targets(value)
        } else {
            Ok(vec![value])
        }
    }
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

pub(super) struct EditNames<'a> {
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

    // Outside edits defer the full listing to stop unless a target also has a checkout name.
    fn validate(&self, root: &Path) -> io::Result<bool> {
        let mut seen = Readings::default();
        let mut in_root = false;
        for name in self.names {
            validate_target(&self.cwd.join(name))?;
            if !in_root {
                self.readings(name, &mut seen, |p| {
                    in_root |= checkout_names(root, p).next().is_some();
                })?;
            }
        }
        Ok(in_root)
    }

    pub(super) fn select<'a>(
        &self,
        root: &Path,
        names: &'a [String],
        base: &'a crate::ledger::Counts,
    ) -> io::Result<HashSet<&'a Path>> {
        let mut candidates: HashSet<_> = names
            .iter()
            .chain(base.keys().map(|(_, path)| path))
            .map(Path::new)
            .collect();
        let mut selected = HashSet::new();
        let mut written = HashSet::new();
        self.visit(|path| {
            selected.extend(
                checkout_names(root, path).filter_map(|name| candidates.take(name.as_path())),
            );
            written.extend(crate::files::identity(path));
        })?;
        if !written.is_empty() {
            selected.extend(names.iter().map(Path::new).filter(|name| {
                candidates.contains(name)
                    && crate::files::identity(&root.join(name))
                        .is_some_and(|id| written.contains(&id))
            }));
        }
        Ok(selected)
    }

    fn visit(&self, mut visit: impl FnMut(&Path)) -> io::Result<()> {
        let mut seen = Readings::default();
        for name in self.names {
            self.readings(name, &mut seen, &mut visit)?;
        }
        Ok(())
    }

    // Every path a target may go by: the caller matches departed files by name and written ones by what the path leads to.
    fn readings(
        &self,
        name: &str,
        seen: &mut Readings,
        mut visit: impl FnMut(&Path),
    ) -> io::Result<()> {
        let path = self.cwd.join(name);
        let resolved = resolve(&path, &mut seen.physical)?;
        for (normalized, seen) in self.normalized(name).zip(&mut seen.followed) {
            // A writer that normalizes first still follows the links left in the result; where they lead nowhere it wrote nothing.
            if normalized != path
                && let Ok(Some(followed)) = resolve(&normalized, seen)
                && Some(&followed) != resolved.as_ref()
            {
                visit(&followed);
            }
            // Git or the ledger may hold the name from before a directory on it became a link.
            if Some(&normalized) != resolved.as_ref() {
                visit(&normalized);
            }
        }
        if let Some(resolved) = resolved {
            visit(&resolved);
        }
        Ok(())
    }
}

#[derive(Default)]
struct Readings {
    physical: crate::files::Seen,
    followed: [crate::files::Seen; 2],
}

fn unresolved(e: &io::Error) -> io::Error {
    io::Error::new(e.kind(), format!("Cannot resolve edit target: {e}"))
}

fn resolve(path: &Path, seen: &mut crate::files::Seen) -> io::Result<Option<PathBuf>> {
    match crate::files::resolve(path, seen) {
        Ok(p) => Ok(Some(p)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(unresolved(&e)),
    }
}

fn same_file(path: &Path, ledger: &Path, metadata: Option<&fs::Metadata>) -> bool {
    #[cfg(unix)]
    if let (Ok(a), Some(b)) = (fs::metadata(path), metadata) {
        return crate::files::same(&a, b);
    }
    #[cfg(not(unix))]
    if let (Ok(target), Ok(existing)) = (fs::canonicalize(path), fs::canonicalize(ledger)) {
        return target == existing;
    }
    path == ledger
}
fn checkout_names<'a>(root: &'a Path, path: &'a Path) -> impl Iterator<Item = PathBuf> + 'a {
    let direct = path.strip_prefix(root).ok().map(Path::to_owned);
    // Preserve the suffix at each directory alias: the ledger can still name a departed file through it.
    let aliases = path
        .ancestors()
        .filter(|prefix| prefix.is_symlink())
        .flat_map(move |prefix| {
            let suffix = path.strip_prefix(prefix).expect("ancestor is a prefix");
            alias_spellings(prefix).filter_map(move |target| {
                target.strip_prefix(root).ok().map(|name| name.join(suffix))
            })
        });
    direct.into_iter().chain(aliases)
}

fn alias_spellings(link: &Path) -> impl Iterator<Item = PathBuf> {
    let final_target = fs::canonicalize(link).ok();
    let mut pending = final_target.as_ref().map(|_| link.to_owned());
    let intermediate = std::iter::from_fn(move || {
        let path = pending.take()?;
        let mut prefix = PathBuf::new();
        let mut parts = path.components();
        while let Some(part) = parts.next() {
            prefix.push(part);
            if let Ok(target) = fs::read_link(&prefix) {
                let parent = fs::canonicalize(prefix.parent()?).ok()?;
                let suffix = parts.as_path();
                pending = Some(parent.join(target).join(suffix));
                return Some(parent.join(part).join(suffix));
            }
        }
        None
    });
    // The filesystem limits link traversal, and a link can change after canonicalization.
    intermediate
        .take(40)
        .filter(|path| !path.components().any(|part| part == Component::ParentDir))
        .chain(final_target)
}
