use std::{
    env, fs, io,
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
    let Some((parent, name)) = path.parent().zip(path.file_name()) else {
        return fs::canonicalize(path);
    };
    match fs::canonicalize(parent) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => existing(parent),
        found => found,
    }
    .map(|dir| dir.join(name))
}

fn locate(dir: &Path, what: &str) -> io::Result<PathBuf> {
    let output = git(dir, &["rev-parse", what])?;
    std::str::from_utf8(output.strip_suffix(b"\n").unwrap_or(&output))
        .map(PathBuf::from)
        .map_err(io::Error::other)
}

// Found once per run: a repository that appears under the tree later must not move the ledger.
pub(crate) fn root() -> io::Result<PathBuf> {
    fs::canonicalize(locate(&env::current_dir()?, "--show-toplevel")?)
}

// Held while the ledger is read and replaced, so two runs cannot interleave.
pub(crate) fn lock(root: &Path) -> io::Result<fs::File> {
    // The lock follows the tree that holds the ledger, whichever repository the caller stands in.
    let path = locate(root, "--absolute-git-dir")?.join("blackhole.lock");
    let mut open = fs::OpenOptions::new();
    // Creation never follows a link and an existing lock is opened without creating, so no file appears outside the repository.
    let file = match open.write(true).create_new(true).open(&path) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            // Nothing is written through the handle, so a link swapped in after this look loses no data.
            if !fs::symlink_metadata(&path)?.is_file() {
                return Err(io::Error::other(format!(
                    "{} must be a regular file",
                    path.display()
                )));
            }
            open.create_new(false).open(&path)?
        }
        other => other?,
    };
    file.lock()?;
    Ok(file)
}

// A file can go away or turn into a directory between the listing and the read, and then it has nothing to check.
// A link to a file is read under the link's name, or a link could hide what kind of file it is.
pub(crate) fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let found = fs::metadata(path).and_then(|meta| {
        if meta.is_file() {
            fs::read(path).map(Some)
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

// A policy file is read only as a regular file: a link would point it outside the repository.
pub(crate) fn policy(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => fs::read(path).map(Some),
        Ok(_) => Err(io::Error::other(format!(
            "{} must be a regular file",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub(crate) fn list(root: &Path, paths: &[PathBuf]) -> io::Result<(Vec<String>, Vec<PathBuf>)> {
    let cwd = env::current_dir()?;
    let paths = paths
        .iter()
        .map(|p| resolve(&cwd.join(p)))
        .collect::<io::Result<Vec<_>>>()?;
    let mut selected = Vec::new();
    let names = git(root, &["ls-files", "-co", "--exclude-standard", "-z"])?;
    for bytes in names.split(|&b| b == 0).filter(|name| !name.is_empty()) {
        let name = std::str::from_utf8(bytes).map_err(|_| {
            io::Error::other(format!("path is not valid UTF-8: {}", bytes.escape_ascii()))
        })?;
        let file = root.join(name);
        if paths.is_empty() || paths.iter().any(|path| file.starts_with(path)) {
            selected.push(name.to_owned());
        }
    }
    selected.sort_unstable();
    selected.dedup();
    Ok((selected, paths))
}
