use crate::engine;
use globset::{Glob, GlobMatcher, GlobSet, GlobSetBuilder};
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

pub(crate) struct FolderBoundary {
    pub from: GlobMatcher,
    pub deny: GlobSet,
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

pub struct Config {
    pub(crate) lcov: Option<String>,
    pub(crate) exclude: GlobSet,
    pub(crate) guard: GlobSet,
    max_lines: Option<NonZeroUsize>,
    banned_imports: Vec<String>,
    folders: Vec<FolderBoundary>,
}

impl Config {
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
            bans: &self.banned_imports,
            folders: &self.folders,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ImportPolicy<'a> {
    pub bans: &'a [String],
    pub folders: &'a [FolderBoundary],
}

fn globs(patterns: Vec<String>) -> io::Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        set.add(Glob::new(&pattern).map_err(io::Error::other)?);
    }
    set.build().map_err(io::Error::other)
}

/// # Errors
/// Returns an error if the repository configuration is invalid.
pub fn parse(bytes: &[u8]) -> io::Result<Config> {
    let settings: Settings = toml::from_slice(bytes)
        .map_err(|e| io::Error::other(format!("blackhole.toml: {}", e.message().escape_debug())))?;
    let mut folders = Vec::new();
    for folder in settings.boundaries.folders {
        if folder.from.is_empty()
            || folder.deny.is_empty()
            || folder.deny.iter().any(String::is_empty)
        {
            return Err(io::Error::other("blackhole.toml: empty folder boundary"));
        }
        folders.push(FolderBoundary {
            from: Glob::new(&folder.from)
                .map_err(io::Error::other)?
                .compile_matcher(),
            deny: globs(folder.deny)?,
        });
    }
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
        max_lines: settings.boundaries.max_lines,
        banned_imports: settings.boundaries.banned_imports,
        folders,
    })
}
