use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
};

fn git(dir: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let output = Command::new("git").args(args).current_dir(dir).output()?;
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

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        if part == Component::ParentDir {
            result.pop();
        } else {
            result.push(part);
        }
    }
    result
}

pub(crate) fn list(paths: &[PathBuf]) -> io::Result<(PathBuf, Vec<String>)> {
    let cwd = env::current_dir()?;
    let output = git(&cwd, &["rev-parse", "--show-toplevel"])?;
    let root = std::str::from_utf8(output.strip_suffix(b"\n").unwrap_or(&output))
        .map_err(io::Error::other)?;
    let root = Path::new(root);
    let paths: Vec<_> = paths.iter().map(|p| normalize(&cwd.join(p))).collect();
    let mut matched = vec![false; paths.len()];
    let mut selected = Vec::new();
    let names = git(root, &["ls-files", "-co", "--exclude-standard", "-z"])?;
    for bytes in names.split(|&b| b == 0).filter(|name| !name.is_empty()) {
        let name = std::str::from_utf8(bytes).map_err(io::Error::other)?;
        let file = root.join(name);
        match fs::symlink_metadata(&file) {
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
    Ok((root.to_owned(), selected))
}
