use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

fn git(dir: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let mut command = Command::new("git");
    // The caller's git variables must not point discovery at another repository or index.
    for (name, _) in env::vars_os() {
        if name.as_encoded_bytes().starts_with(b"GIT_") {
            command.env_remove(name);
        }
    }
    let output = command.args(args).current_dir(dir).output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("git failed")
                .to_owned(),
        ));
    }
    Ok(output.stdout)
}

// Links in the directories are followed as the filesystem follows them. The last name stays as written so a link can be selected.
// Directories that are gone stay as written too, so the ledger rows under them can still be selected.
fn resolve(path: &Path) -> io::Result<PathBuf> {
    existing(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

fn existing(path: &Path) -> io::Result<PathBuf> {
    let mut gone = Vec::new();
    let mut at = path;
    let mut found = loop {
        let Some((parent, name)) = at.parent().zip(at.file_name()) else {
            break fs::canonicalize(at)?;
        };
        gone.push(name);
        match fs::canonicalize(parent) {
            // A link that points nowhere is an entry, not a directory that is gone.
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) && fs::symlink_metadata(parent).is_err() =>
            {
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
    let output = git(&env::current_dir()?, &["rev-parse", "--show-toplevel"])?;
    let top = std::str::from_utf8(output.strip_suffix(b"\n").unwrap_or(&output))
        .map_err(io::Error::other)?;
    fs::canonicalize(top)
}

// Held while the ledger is read and replaced, so two runs cannot interleave.
// The lock is on the root itself: a lock file would have to live in the tree or in a git directory that can be anywhere.
pub(crate) fn lock(root: &Path) -> io::Result<fs::File> {
    // shortcut: Windows cannot open a directory this way, lock a file under the root when Windows is supported
    let dir = fs::File::open(root)?;
    dir.lock()?;
    Ok(dir)
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
