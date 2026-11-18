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
fn resolve(path: &Path) -> io::Result<PathBuf> {
    let found = match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent).map(|dir| dir.join(name)),
        _ => fs::canonicalize(path),
    };
    found.map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

fn locate(cwd: &Path, what: &str) -> io::Result<PathBuf> {
    let output = git(cwd, &["rev-parse", what])?;
    std::str::from_utf8(output.strip_suffix(b"\n").unwrap_or(&output))
        .map(PathBuf::from)
        .map_err(io::Error::other)
}

// Held while the ledger is read and replaced, so two runs cannot interleave.
pub(crate) fn lock() -> io::Result<fs::File> {
    let dir = locate(&env::current_dir()?, "--absolute-git-dir")?;
    let file = fs::File::create(dir.join("blackhole.lock"))?;
    file.lock()?;
    Ok(file)
}

pub(crate) fn list(paths: &[PathBuf]) -> io::Result<(PathBuf, Vec<String>)> {
    let cwd = env::current_dir()?;
    let root = fs::canonicalize(locate(&cwd, "--show-toplevel")?)?;
    let paths = paths
        .iter()
        .map(|p| resolve(&cwd.join(p)))
        .collect::<io::Result<Vec<_>>>()?;
    let mut matched = vec![false; paths.len()];
    let mut selected = Vec::new();
    let names = git(&root, &["ls-files", "-co", "--exclude-standard", "-z"])?;
    for bytes in names.split(|&b| b == 0).filter(|name| !name.is_empty()) {
        let name = std::str::from_utf8(bytes).map_err(|_| {
            io::Error::other(format!("path is not valid UTF-8: {}", bytes.escape_ascii()))
        })?;
        let file = root.join(name);
        // A link to a file is read under the link's name, or a link could hide what kind of file it is.
        match fs::metadata(&file) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
        let mut include = paths.is_empty();
        for (path, hit) in paths.iter().zip(&mut matched) {
            if file.starts_with(path) {
                *hit = true;
                include = true;
            }
        }
        if include {
            selected.push(name.to_owned());
        }
    }
    if let Some(i) = matched.iter().position(|&hit| !hit) {
        return Err(io::Error::other(format!(
            "no files match {}",
            paths[i].display().to_string().escape_default()
        )));
    }
    selected.sort_unstable();
    selected.dedup();
    Ok((root, selected))
}
