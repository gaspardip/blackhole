use crate::engine;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::{io, num::NonZeroUsize};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    exclude: Vec<String>,
    boundaries: Boundaries,
    guard: Guard,
    coverage: Option<Coverage>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct Boundaries {
    max_lines: Option<NonZeroUsize>,
    banned_imports: Vec<String>,
    folders: Vec<Folder>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Folder {
    from: String,
    deny: Vec<String>,
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

#[derive(Default)]
pub struct Config {
    pub(crate) lcov: Option<String>,
    pub(crate) exclude: GlobSet,
    pub(crate) guard: GlobSet,
    max_lines: Option<NonZeroUsize>,
    banned_imports: Vec<String>,
    folder_origins: GlobSet,
    folder_denies: Vec<GlobSet>,
}

impl Config {
    pub(crate) fn for_banned_imports(settings: engine::Settings, bans: &[String]) -> Self {
        Self {
            max_lines: settings.max_lines,
            banned_imports: bans.to_vec(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn engine_settings(&self) -> engine::Settings {
        engine::Settings {
            max_lines: self.max_lines,
        }
    }

    #[must_use]
    pub fn banned_imports(&self) -> &[String] {
        &self.banned_imports
    }

    pub(crate) fn import_policy(&self) -> ImportPolicy<'_> {
        ImportPolicy {
            bans: self.banned_imports(),
            folder_origins: &self.folder_origins,
            folder_denies: &self.folder_denies,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ImportPolicy<'a> {
    pub bans: &'a [String],
    pub folder_origins: &'a GlobSet,
    pub folder_denies: &'a [GlobSet],
}

fn globs(patterns: impl IntoIterator<Item = Result<Glob, globset::Error>>) -> io::Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        set.add(pattern.map_err(io::Error::other)?);
    }
    set.build().map_err(io::Error::other)
}

/// # Errors
/// Returns an error if the repository configuration is invalid.
pub fn parse(bytes: &[u8]) -> io::Result<Config> {
    let settings: Settings = toml::from_slice(bytes)
        .map_err(|e| io::Error::other(format!("blackhole.toml: {}", e.message().escape_debug())))?;
    let mut origins = Vec::new();
    let mut denies = Vec::new();
    for folder in settings.boundaries.folders {
        if folder.from.is_empty()
            || folder.deny.is_empty()
            || folder.deny.iter().any(String::is_empty)
        {
            return Err(io::Error::other("blackhole.toml: empty folder boundary"));
        }
        origins.push(Glob::new(&folder.from).map_err(io::Error::other)?);
        denies.push(globs(folder.deny.iter().map(|pattern| Glob::new(pattern)))?);
    }
    let folder_origins = globs(origins.into_iter().map(Ok))?;
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
        exclude: globs(settings.exclude.iter().map(|pattern| Glob::new(pattern)))?,
        guard: globs(
            settings
                .guard
                .paths
                .iter()
                .map(|pattern| Glob::new(pattern)),
        )?,
        max_lines: settings.boundaries.max_lines,
        banned_imports: settings.boundaries.banned_imports,
        folder_origins,
        folder_denies: denies,
    })
}
