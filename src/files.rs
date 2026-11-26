use std::{
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

fn git(dir: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    crate::git::output(crate::git::command().args(args).current_dir(dir))
}

// Links in the directories are followed as the filesystem follows them. The last name stays as written so a link can be selected.
// Directories that are gone stay as written too, so the ledger rows under them can still be selected.
fn resolve(path: &Path) -> io::Result<PathBuf> {
    existing(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

fn existing(path: &Path) -> io::Result<PathBuf> {
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
        match fs::canonicalize(parent) {
            // A link that points nowhere is an entry, and a name that cannot be looked at is an error: neither is a directory that is gone.
            Err(e) if absent(&e) && fs::symlink_metadata(parent).is_err_and(|e| absent(&e)) => {
                at = parent;
            }
            dir => break dir?,
        }
    };
    found.extend(gone.into_iter().rev());
    Ok(found)
}

// Found once per run: a repository that appears under the tree later must not move the ledger.
pub(crate) fn root() -> io::Result<PathBuf> {
    let cwd = env::current_dir()?;
    let output = git(&cwd, &["rev-parse", "--show-toplevel"])?;
    let top = std::str::from_utf8(output.strip_suffix(b"\n").unwrap_or(&output))
        .map_err(io::Error::other)?;
    let top = fs::canonicalize(top)?;
    // `core.worktree` can send git anywhere, so its answer must be where the nearest `.git` sits.
    for dir in cwd.ancestors() {
        match dir.join(".git").symlink_metadata() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            Ok(_) if fs::canonicalize(dir)? == top => return Ok(top),
            Ok(_) => break,
        }
    }
    Err(io::Error::other("checkout root mismatch"))
}

// Held while the ledger is read and replaced, so two runs cannot interleave.
// The lock is on the root itself: a lock file would have to live in the tree or in a git directory that can be anywhere.
pub(crate) fn lock(root: &Path) -> io::Result<fs::File> {
    // shortcut: Windows cannot open a directory this way, lock a file under the root when Windows is supported
    let dir = fs::File::open(root)?;
    dir.lock()?;
    Ok(dir)
}

pub(crate) fn is_file(path: &Path) -> io::Result<bool> {
    Ok(with_file(path, || Ok(()))?.is_some())
}

pub(crate) fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    with_file(path, || fs::read(path))
}

// A file can go away or turn into a directory between the listing and the read, and then it has nothing to check.
// A link to a file is read under the link's name, or a link could hide what kind of file it is.
fn with_file<T>(path: &Path, use_file: impl FnOnce() -> io::Result<T>) -> io::Result<Option<T>> {
    let found = fs::metadata(path).and_then(|meta| {
        if meta.is_file() {
            use_file().map(Some)
        } else {
            Ok(None)
        }
    });
    match found {
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::NotADirectory
                    | io::ErrorKind::IsADirectory
            ) =>
        {
            Ok(None)
        }
        other => other,
    }
}

#[cfg(unix)]
fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
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
    let named = match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        other => other?,
    };
    if named.is_file() {
        // shortcut: a link swapped in after the look is still opened, and read only when it leads to the same file, and opening a device can have an effect of its own; open without following links once a crate that names the flag is approved
        let mut file = fs::File::open(path)?;
        // The name can become a link between the look and the open, so the opened file must be the one that was looked at.
        if same(&named, &file.metadata()?) {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            return Ok(Some(bytes));
        }
    }
    Err(io::Error::other(format!(
        "{} must be a regular file",
        path.display()
    )))
}

pub(crate) fn list(root: &Path, paths: &[PathBuf]) -> io::Result<(Vec<String>, Vec<PathBuf>)> {
    let cwd = env::current_dir()?;
    let paths = paths
        .iter()
        .map(|p| resolve(&cwd.join(p)))
        .collect::<io::Result<Vec<_>>>()?;
    let mut listed = Vec::new();
    let names = git(root, &["ls-files", "-co", "--exclude-standard", "-z"])?;
    for bytes in names.split(|&b| b == 0).filter(|name| !name.is_empty()) {
        let name = std::str::from_utf8(bytes).map_err(|_| {
            io::Error::other(format!("path is not valid UTF-8: {}", bytes.escape_ascii()))
        })?;
        listed.push(name.to_owned());
    }
    listed.sort_unstable();
    listed.dedup();
    Ok((listed, paths))
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
                let target = fs::read_link(&real)?;
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
            Err(e) => return Err(io::Error::new(e.kind(), format!("{name:?}: {e}"))),
        }
    }
    Ok(paths)
}
