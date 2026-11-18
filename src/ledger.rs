use crate::engine::{Finding, Rule};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    io,
};

pub type Key = (String, String);
pub type Counts = BTreeMap<Key, usize>;
const HEADER: &str = "blackhole-ledger 1\n";

/// # Errors
/// Reports the first malformed line.
pub fn parse(bytes: &[u8], rules: &[Rule]) -> io::Result<Counts> {
    let mut lines = bytes.split_inclusive(|&b| b == b'\n');
    if lines.next() != Some(HEADER.as_bytes()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ledger line 1: invalid or missing header",
        ));
    }
    let mut rows = Counts::new();
    for (i, raw) in lines.enumerate() {
        let error = |reason| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ledger line {}: {reason}", i + 2),
            )
        };
        let raw = raw
            .strip_suffix(b"\n")
            .ok_or_else(|| error("missing newline"))?;
        let line = std::str::from_utf8(raw).map_err(|_| error("invalid UTF-8"))?;
        let mut fields = line.split('\t');
        let (Some(rule), Some(count), Some(path), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(error("expected three fields"));
        };
        if !rules.iter().any(|r| r.id == rule) {
            return Err(error("unknown rule"));
        }
        if count.starts_with('0') || !count.bytes().all(|b| b.is_ascii_digit()) {
            return Err(error("invalid count"));
        }
        let count = count.parse().map_err(|_| error("invalid count"))?;
        if invalid_path(path) {
            return Err(error("invalid path"));
        }
        let key = (rule.to_owned(), path.to_owned());
        if rows.last_key_value().is_some_and(|(prev, _)| prev >= &key) {
            return Err(error("expected strictly increasing keys"));
        }
        rows.insert(key, count);
    }
    Ok(rows)
}

fn invalid_path(path: &str) -> bool {
    path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', '\r', '\t', '\n'])
        || path.split('/').any(|p| p == "..")
}

/// # Errors
/// Rejects paths that cannot be represented in the ledger.
pub fn write(rows: &Counts) -> io::Result<String> {
    let mut text = HEADER.to_owned();
    for ((rule, path), n) in rows {
        if invalid_path(path) {
            return Err(io::Error::other("invalid ledger path"));
        }
        writeln!(text, "{rule}\t{n}\t{path}").unwrap();
    }
    Ok(text)
}

#[must_use]
pub fn counts(findings: &[Finding]) -> Counts {
    let mut rows = Counts::new();
    for f in findings {
        *rows.entry((f.rule.to_owned(), f.path.clone())).or_default() += 1;
    }
    rows
}

pub struct Difference {
    pub over: Counts,
    pub stale: Counts,
}

#[must_use]
pub fn exact(base: &Counts, actual: &Counts, selected: Option<&BTreeSet<&str>>) -> Difference {
    let included = |key: &Key| selected.is_none_or(|paths| paths.contains(key.1.as_str()));
    let mut over = Counts::new();
    let mut stale = Counts::new();
    for (key, &n) in actual {
        if included(key) && n > base.get(key).copied().unwrap_or(0) {
            over.insert(key.clone(), n);
        }
    }
    for (key, &n) in base {
        if included(key) && actual.get(key).copied().unwrap_or(0) < n {
            stale.insert(key.clone(), n);
        }
    }
    Difference { over, stale }
}

#[must_use]
pub fn growth(base: &Counts, head: &Counts, renames: &[(&str, &str)]) -> Counts {
    let renames: BTreeMap<_, _> = renames.iter().copied().collect();
    let mut moved = Counts::new();
    for ((rule, path), &n) in base {
        let dest = renames.get(path.as_str()).copied().unwrap_or(path);
        let allowance = moved.entry((rule.clone(), dest.to_owned())).or_default();
        *allowance = allowance.saturating_add(n);
    }
    exact(&moved, head, None).over
}
