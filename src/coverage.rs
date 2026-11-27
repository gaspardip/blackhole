mod lcov;

use crate::{
    files,
    git::{self, Batch, Tree},
    verdict::{Change, Kind},
};
use globset::GlobSet;
use std::{fs, io, ops::Range, path::Path};

pub(crate) fn relative_path(value: &str) -> Option<&str> {
    let path = value.strip_prefix("./").unwrap_or(value);
    let drive = matches!(path.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    (!drive && !crate::ledger::invalid_path(path)).then_some(path)
}

pub(crate) struct Changed<'a> {
    path: &'a str,
    lines: usize,
    hunks: Vec<Range<usize>>,
}

pub(crate) enum Delta<'a> {
    Lines(Vec<Changed<'a>>),
    TooLarge(&'a str),
}

const EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs", "vue", "svelte", "astro", "rs", "py",
    "go", "rb", "java", "kt", "kts", "cs", "c", "h", "cc", "cpp", "cxx", "hh", "hpp", "hxx", "m",
    "mm", "swift", "php", "sh", "bash", "dash", "ksh", "scala", "lua", "dart", "r", "ex", "exs",
    "erl", "clj", "groovy", "pl", "pm",
];

pub(crate) fn source_name(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or_default();
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn source(path: &str, bytes: &[u8]) -> bool {
    let path = path.to_ascii_lowercase();
    let mut parts = path.rsplit('/');
    let name = parts.next().unwrap_or_default();
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let suffix = stem.rsplit_once('.').map(|(_, suffix)| suffix);
    let test = parts.any(|dir| matches!(dir, "__tests__" | "tests" | "test" | "spec"))
        || matches!(suffix, Some("test" | "spec"))
        || (ext == "go" && stem.ends_with("_test"))
        || (ext == "py" && (stem.starts_with("test_") || stem.ends_with("_test")))
        || (ext == "rb" && (stem.ends_with("_spec") || stem.ends_with("_test")));
    let declaration = matches!(ext, "ts" | "cts" | "mts") && suffix == Some("d");
    !test && !declaration && (source_name(&path) || bytes.starts_with(b"#!"))
}

fn fits_text_diff(old_size: usize, new_size: usize) -> bool {
    // Git's xdi_diff checks MAX_XDIFF_SIZE from xdiff-interface.h for each input.
    old_size.max(new_size) <= 1023 * 1024 * 1024
}

fn hunks(old: &str, new: &str) -> io::Result<Vec<Range<usize>>> {
    let patch = git::output(git::command().args([
        "diff",
        "--no-relative",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--text",
        "--unified=0",
        "--inter-hunk-context=0",
        old,
        new,
        "--",
    ]))?;
    let mut ranges = Vec::new();
    for line in patch
        .split(|&c| c == b'\n')
        .filter(|l| l.starts_with(b"@@ "))
    {
        let field = line
            .split(|&c| c == b' ')
            .nth(2)
            .and_then(|f| f.strip_prefix(b"+"))
            .ok_or_else(|| io::Error::other("invalid git hunk"))?;
        let field = std::str::from_utf8(field).map_err(io::Error::other)?;
        let (start, count) = field.split_once(',').unwrap_or((field, "1"));
        let start: usize = start.parse().map_err(io::Error::other)?;
        let count: usize = count.parse().map_err(io::Error::other)?;
        if count > 0 {
            ranges.push(
                start
                    ..start
                        .checked_add(count)
                        .ok_or_else(|| io::Error::other("git line overflow"))?,
            );
        }
    }
    Ok(ranges)
}

fn source_blob<'a>(
    id: Option<&'a str>,
    path: &str,
    batch: &mut Batch,
) -> io::Result<Option<(&'a str, Vec<u8>)>> {
    let Some(id) = id else {
        return Ok(None);
    };
    let bytes = batch.read(id)?;
    Ok(source(path, &bytes).then_some((id, bytes)))
}

pub(crate) fn changed<'a>(
    changes: &'a [Change],
    excludes: &GlobSet,
    base: &Tree,
    head: &Tree,
    batch: &mut Batch,
) -> io::Result<Delta<'a>> {
    let mut delta = Vec::new();
    for change in changes {
        let path = change.path.as_str();
        if matches!(change.kind, Kind::Deleted) || excludes.is_match(path) {
            continue;
        }
        let Some((new, bytes)) = source_blob(head.blob(path, batch)?, path, batch)? else {
            if source(path, &[]) {
                return Err(crate::red(format!(
                    "coverage: {} has no source blob; make it a regular file",
                    crate::diagnostic_text(path)
                )));
            }
            continue;
        };
        let new_size = bytes.len();
        let lines = bytes.split_inclusive(|&b| b == b'\n').count();
        drop(bytes);
        if lines == 0 {
            continue;
        }
        let old_path = match &change.kind {
            Kind::Moved(from) => from,
            _ => path,
        };
        let old = if excludes.is_match(old_path) {
            None
        } else {
            source_blob(base.historical_blob(old_path, batch)?, old_path, batch)?
                .map(|(id, bytes)| (id, bytes.len()))
        };
        let hunks = match old {
            Some((id, _)) if id == new => continue,
            Some((_, old_size)) if !fits_text_diff(old_size, new_size) => {
                return Ok(Delta::TooLarge(path));
            }
            Some((id, _)) => hunks(id, new)?,
            None => std::iter::once(1..lines + 1).collect(),
        };
        if !hunks.is_empty() {
            delta.push(Changed { path, lines, hunks });
        }
    }
    Ok(Delta::Lines(delta))
}

pub(crate) fn check(
    delta: &[Changed<'_>],
    artifacts: Option<&Path>,
    head: &str,
    lcov: &str,
) -> io::Result<()> {
    let root = fs::canonicalize(artifacts.ok_or_else(|| {
        io::Error::other("trusted report missing; supply --coverage-dir for this head")
    })?)?;
    let path = root.join(head).join(lcov);
    let actual = fs::canonicalize(&path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("missing or inaccessible {lcov:?} for {head}: {e}"),
        )
    })?;
    // The producer owns these directories. A link must never select a different revision or a checkout.
    if actual.starts_with(git::repository_root()?) || actual != path {
        return Err(io::Error::other(
            "report must be outside the checkout at its exact head path",
        ));
    }
    let bytes = files::policy(&path)?.ok_or_else(|| io::Error::other("missing LCOV file"))?;
    let report = lcov::parse(std::str::from_utf8(&bytes).map_err(io::Error::other)?)?;
    for file in delta {
        let records = report
            .get(file.path)
            .ok_or_else(|| io::Error::other(format!("missing LCOV record for {:?}", file.path)))?;
        if records
            .last_key_value()
            .is_some_and(|(&line, _)| line > file.lines)
        {
            return Err(io::Error::other(format!(
                "line beyond end of {:?}",
                file.path
            )));
        }
        for range in &file.hunks {
            if let Some((line, _)) = records.range(range.clone()).find(|(_, hit)| !**hit) {
                return Err(io::Error::other(format!(
                    "{}:{line}: changed line has zero hits",
                    file.path
                )));
            }
        }
    }
    Ok(())
}

#[test]
fn both_blobs_must_fit_gits_text_diff_limit() {
    let limit = 1_072_693_248;
    assert!(fits_text_diff(limit, limit));
    assert!(!fits_text_diff(limit + 1, 0), "oversize base blob");
    assert!(!fits_text_diff(0, limit + 1), "oversize head blob");
}
