use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::io;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    exclude: Vec<String>,
    guard: Guard,
    coverage: Option<Coverage>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Guard {
    paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    lcov: String,
}

pub(crate) struct Config {
    pub(crate) lcov: Option<String>,
    pub(crate) exclude: GlobSet,
    pub(crate) guard: GlobSet,
}

fn globs(patterns: Vec<String>) -> io::Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        set.add(Glob::new(&pattern).map_err(io::Error::other)?);
    }
    set.build().map_err(io::Error::other)
}

pub(crate) fn parse(bytes: &[u8]) -> io::Result<Config> {
    let settings: Settings = toml::from_slice(bytes)
        .map_err(|e| io::Error::other(format!("blackhole.toml: {}", e.message().escape_debug())))?;
    let lcov = settings
        .coverage
        .map(|coverage| {
            crate::coverage::relative_path(&coverage.lcov)
                .map(str::to_owned)
                .ok_or_else(|| {
                    io::Error::other(
                        "blackhole.toml: coverage.lcov must be a repository-relative path",
                    )
                })
        })
        .transpose()?;
    Ok(Config {
        lcov,
        exclude: globs(settings.exclude)?,
        guard: globs(settings.guard.paths)?,
    })
}
