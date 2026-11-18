use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::{fs, io, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    exclude: Vec<String>,
}

pub(crate) fn parse(bytes: &[u8]) -> io::Result<GlobSet> {
    let config: Config = toml::from_slice(bytes)
        .map_err(|e| io::Error::other(format!("blackhole.toml: {}", e.message().escape_debug())))?;
    let mut globs = GlobSetBuilder::new();
    for pattern in config.exclude {
        globs.add(Glob::new(&pattern).map_err(io::Error::other)?);
    }
    globs.build().map_err(io::Error::other)
}

pub(crate) fn read_optional(path: &Path) -> io::Result<Vec<u8>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => fs::read(path),
        Ok(_) => Err(io::Error::other(format!(
            "{} must be a regular file",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}
