use std::{
    collections::{BTreeSet, HashMap, HashSet},
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

fn git(dir: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    crate::git::output(crate::git::command().args(args).current_dir(dir))
}

// Cache one parent so a batch stays bounded even under a long directory path.
pub(crate) type Seen = Option<(PathBuf, PathBuf)>;

// Links in the directories are followed as the filesystem follows them. The last name stays as written so a link can be selected.
// Directories that are gone stay as written too, so the ledger rows under them can still be selected.
pub(crate) fn resolve(path: &Path, seen: &mut Seen) -> io::Result<PathBuf> {
    let absent = |e: &io::Error| {
        matches!(
            e.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        )
    };
    let mut gone = Vec::new();
    let mut at = path;
    let mut found = loop {
        let Some((parent, name)) = at.parent().zip(at.file_name()) else {
            break fs::canonicalize(at)?;
        };
        gone.push(name);
        if let Some((dir, real)) = seen
            && dir == parent
        {
            break real.clone();
        }
        match fs::canonicalize(parent) {
            // A link that points nowhere is an entry, and a name that cannot be looked at is an error: neither is a directory that is gone.
            Err(e) if absent(&e) && fs::symlink_metadata(parent).is_err_and(|e| absent(&e)) => {
                at = parent;
            }
            dir => {
                let real = dir?;
                *seen = Some((parent.to_owned(), real.clone()));
                break real;
            }
        }
    };
    found.extend(gone.into_iter().rev());
    Ok(found)
}

// Found once per run: a repository that appears under the tree later must not move the ledger.
pub(crate) fn root() -> io::Result<PathBuf> {
    let cwd = env::current_dir()?;
    let dir = nearest_checkout(&cwd)?.ok_or_else(|| crate::usage("not inside a git checkout"))?;
    let output = git(&cwd, &["rev-parse", "--show-toplevel"])?;
    let top = std::str::from_utf8(output.strip_suffix(b"\n").unwrap_or(&output))
        .map_err(io::Error::other)?;
    let top = fs::canonicalize(top)?;
    // `core.worktree` can send git anywhere, so its answer must be where the nearest `.git` sits.
    if fs::canonicalize(dir)? == top {
        return Ok(top);
    }
    Err(crate::red(format!(
        "checkout root mismatch: git reports {}, but the nearest .git is in {}; make core.worktree name this checkout or remove the stray .git",
        crate::diagnostic_text(&top.to_string_lossy()),
        crate::diagnostic_text(&dir.to_string_lossy()),
    )))
}

// Only a search that finds no `.git` at all establishes that; a failed search or a broken marker may still be a checkout.
pub(crate) fn outside_checkout() -> bool {
    env::current_dir().is_ok_and(|cwd| nearest_checkout(&cwd).is_ok_and(|dir| dir.is_none()))
}

fn nearest_checkout(cwd: &Path) -> io::Result<Option<&Path>> {
    for dir in cwd.ancestors() {
        match dir.join(".git").symlink_metadata() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            Ok(_) => return Ok(Some(dir)),
        }
    }
    Ok(None)
}

// Held while the ledger is read and replaced, so two runs cannot interleave.
// The lock is on the root itself: a lock file would have to live in the tree or in a git directory that can be anywhere.
pub(crate) fn lock(root: &Path) -> io::Result<fs::File> {
    // shortcut: Windows cannot open a directory this way, lock a file under the root when Windows is supported
    let dir = fs::File::open(root)?;
    dir.lock()?;
    Ok(dir)
}

// A link to a file is read under the link's name, or a link could hide what kind of file it is.
pub(crate) fn read(root: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let path = root.join(name);
    crate::deadline::activity(|| format!("reading {}", crate::diagnostic_text(name)));
    let found = fs::symlink_metadata(&path).and_then(|meta| {
        let link = meta.is_symlink();
        let meta = if link { fs::metadata(&path)? } else { meta };
        if meta.is_file() {
            fs::read(&path).map(Some)
        } else if meta.is_dir() && (link || !tracked_file(root, name)?) {
            Ok(None)
        } else {
            Err(io::Error::other("expected a regular file"))
        }
    });
    match found {
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(e) if e.kind() == io::ErrorKind::IsADirectory && !tracked_file(root, name)? => Ok(None),
        other => other,
    }
}

fn tracked_file(root: &Path, name: &str) -> io::Result<bool> {
    let entries = git(
        root,
        &[
            "--literal-pathspecs",
            "ls-files",
            "--stage",
            "-z",
            "--",
            name,
        ],
    )?;
    Ok(entries.split(|&b| b == 0).any(|entry| {
        let mut fields = entry.splitn(2, |&b| b == b'\t');
        fields
            .next()
            .is_some_and(|mode| !mode.starts_with(b"160000 "))
            && fields.next() == Some(name.as_bytes())
    }))
}

// What two names of one file share, wherever a link or a second name leads to it.
#[cfg(unix)]
pub(crate) fn identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path)
        .ok()
        .filter(fs::Metadata::is_file)
        .map(|m| (m.dev(), m.ino()))
}

// shortcut: std gives no file identity elsewhere, so there a second hard link is a different file; compare volume and file index when Windows is supported
#[cfg(not(unix))]
pub(crate) fn identity(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path)
        .ok()
        .filter(|target| target.is_file())
}

#[cfg(unix)]
pub(crate) fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

// shortcut: std gives no file identity elsewhere, so there a link swapped in after the look is followed; compare volume and file index when Windows is supported
#[cfg(not(unix))]
fn same(_: &fs::Metadata, _: &fs::Metadata) -> bool {
    true
}

pub(crate) const LEDGER_PATH: &str = ".blackhole/ledger";
pub(crate) const OBJECTIONS_PATH: &str = ".blackhole/objections";

// A policy file is read only as a regular file: a link would point it outside the repository.
pub(crate) fn policy(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let Some(named) = regular(path)? else {
        return Ok(None);
    };
    // shortcut: a link swapped in after the look is still opened, and read only when it leads to the same file, and opening a device can have an effect of its own; open without following links once a crate that names the flag is approved
    let mut file = fs::File::open(path)?;
    // The name can become a link between the look and the open, so the opened file must be the one that was looked at.
    if same(&named, &file.metadata()?) {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        return Ok(Some(bytes));
    }
    Err(not_regular(path))
}

fn not_regular(path: &Path) -> io::Error {
    crate::red(format!("{} must be a regular file", path.display()))
}

pub(crate) fn regular(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(Some(meta)),
        Ok(_) => Err(not_regular(path)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Default)]
pub(crate) struct Listing<'a> {
    pub names: Vec<String>,
    pub rows: Option<HashSet<&'a str>>,
    pub unmatched: Option<PathBuf>,
}

pub(crate) fn list<'a>(
    root: &Path,
    cwd: &Path,
    paths: &[PathBuf],
    rows: impl Iterator<Item = &'a str>,
) -> io::Result<Listing<'a>> {
    if paths.is_empty() {
        return Ok(Listing {
            names: listed(root, [Path::new(".")])?,
            ..Listing::default()
        });
    }
    let mut queries = BTreeSet::new();
    let mut wanted = HashMap::new();
    let mut unmatched = None;
    let mut seen = None;
    for path in paths {
        let named = cwd.join(path);
        let real = resolve(&named, &mut seen).map_err(|e| {
            crate::unresolvable(&crate::diagnostic_text(&named.to_string_lossy()), &e)
        })?;
        let literal = named.strip_prefix(root).ok().filter(|p| {
            !p.components()
                .any(|part| part == std::path::Component::ParentDir)
        });
        if literal.is_some() || real.starts_with(root) {
            queries.extend(literal.map(|p| p.components().collect::<PathBuf>()));
            queries.extend(real.strip_prefix(root).ok().map(Path::to_owned));
            wanted.insert(real, false);
        } else {
            unmatched.get_or_insert(real);
        }
    }
    let mut select = |name: &str| {
        let named = root.join(name);
        let real = resolve(&named, &mut seen).ok();
        let mut hit = false;
        for path in named
            .ancestors()
            .chain(real.iter().flat_map(|p| p.ancestors()))
        {
            if let Some(matched) = wanted.get_mut(path) {
                *matched = true;
                hit = true;
            }
        }
        hit
    };
    let rows: HashSet<_> = rows.filter(|name| select(name)).collect();
    queries.extend(rows.iter().map(PathBuf::from));
    let names = listed(root, queries.iter().map(PathBuf::as_path))?;
    for name in &names {
        select(name);
    }
    let unmatched = unmatched.or_else(|| {
        wanted
            .into_iter()
            .filter_map(|(path, hit)| (!hit).then_some(path))
            .min()
    });
    Ok(Listing {
        names,
        rows: Some(rows),
        unmatched,
    })
}

fn listed<'a>(root: &Path, paths: impl IntoIterator<Item = &'a Path>) -> io::Result<Vec<String>> {
    listed_matching(root, paths, |_| true)
}

pub(crate) fn listed_matching<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
    mut matches: impl FnMut(&Path) -> bool,
) -> io::Result<Vec<String>> {
    let paths: Vec<_> = paths
        .into_iter()
        .map(|path| {
            if path == Path::new("") {
                Path::new(".")
            } else {
                path
            }
        })
        .collect();
    // Use one argument budget for both short names and paths near the filesystem limit.
    let longest = paths.iter().map(|p| p.as_os_str().len()).max().unwrap_or(0);
    let batch = (64 * 1024 / (longest + 16)).max(1);
    let mut selected = Vec::new();
    for paths in paths.chunks(batch) {
        let names = crate::git::output(
            crate::git::command()
                .current_dir(root)
                .arg("--literal-pathspecs")
                .args(["ls-files", "-co", "--exclude-standard", "-z", "--"])
                .args(paths),
        )?;
        for name in names.split(|&b| b == 0).filter(|name| !name.is_empty()) {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                if !matches(Path::new(std::ffi::OsStr::from_bytes(name))) {
                    continue;
                }
            }
            let name = std::str::from_utf8(name).map_err(|_| {
                crate::red(format!("path is not valid UTF-8: {}", name.escape_ascii()))
            })?;
            #[cfg(not(unix))]
            if !matches(Path::new(name)) {
                continue;
            }
            selected.push(name.to_owned());
        }
    }
    selected.sort_unstable();
    selected.dedup();
    Ok(selected)
}

pub(crate) fn repository_paths<'a>(
    root: &Path,
    names: impl Iterator<Item = &'a str>,
) -> io::Result<crate::packs::boundary::RepositoryPaths> {
    let mut paths = crate::packs::boundary::RepositoryPaths::default();
    for name in names {
        let real = root.join(name);
        let virtual_path = Path::new("/").join(name);
        match fs::symlink_metadata(&real) {
            Ok(meta) if meta.is_symlink() => {
                let target = fs::read_link(&real).map_err(|e| crate::unreadable(name, &e))?;
                let target = match target.strip_prefix(root) {
                    Ok(local) => {
                        let up: PathBuf =
                            Path::new(name).components().skip(1).map(|_| "..").collect();
                        up.join(local)
                    }
                    Err(_) => target,
                };
                paths.links.insert(virtual_path, target);
            }
            Ok(meta) if meta.is_file() => {
                paths.files.insert(virtual_path);
            }
            Ok(_) => (),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            Err(e) => return Err(crate::unreadable(name, &e)),
        }
    }
    Ok(paths)
}
