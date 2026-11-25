use crate::engine::{Finding, Gate, Rule, registry};
use std::{collections::BTreeMap, fmt::Write, io};

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
        if !crate::engine::can_allow(rules, rule) {
            return Err(error("unknown or prohibited rule"));
        }
        let count = count_of(count).ok_or_else(|| error("invalid count"))?;
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

/// A repository without a ledger allows nothing, which is what an empty one says.
pub(crate) fn load(bytes: Option<Vec<u8>>) -> io::Result<Counts> {
    bytes.map_or_else(|| Ok(Counts::new()), |bytes| parse(&bytes, registry()))
}

// A count has one spelling, so a file has one spelling too.
pub(crate) fn count_of(text: &str) -> Option<usize> {
    (!text.starts_with('0') && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

// Git stores one spelling of a path, so any other spelling is a line no file can match.
pub(crate) fn invalid_path(path: &str) -> bool {
    path.contains(['\\', '\r', '\t', '\n', '\0'])
        || path.split('/').any(|p| matches!(p, "" | "." | ".."))
}

/// # Errors
/// Rejects rows that would not read back as written.
pub fn write(rows: &Counts) -> io::Result<String> {
    let mut text = HEADER.to_owned();
    for ((rule, path), n) in rows {
        writeln!(text, "{rule}\t{n}\t{path}").unwrap();
    }
    if parse(text.as_bytes(), registry()).ok().as_ref() != Some(rows) {
        return Err(io::Error::other("rows cannot be written to the ledger"));
    }
    Ok(text)
}

#[must_use]
pub fn counts(findings: &[Finding]) -> Counts {
    let mut rows = Counts::new();
    for f in findings.iter().filter(|f| f.gate == Gate::Blocking) {
        *rows.entry((f.rule.to_owned(), f.path.clone())).or_default() += 1;
    }
    rows
}

/// The keys of `counts` that hold more than `allowed` gives them.
#[must_use]
pub fn excess(counts: &Counts, allowed: &Counts) -> Counts {
    counts
        .iter()
        .filter(|&(key, &n)| n > allowed.get(key).copied().unwrap_or(0))
        .map(|(key, &n)| (key.clone(), n))
        .collect()
}

/// The base counts, each under the path a rename gave its file.
#[must_use]
pub fn moved(base: &Counts, renames: &[(&str, &str)]) -> Counts {
    let renames: BTreeMap<_, _> = renames.iter().copied().collect();
    let mut moved = Counts::new();
    for ((rule, path), &n) in base {
        let dest = renames.get(path.as_str()).copied().unwrap_or(path);
        let allowance = moved.entry((rule.clone(), dest.to_owned())).or_default();
        *allowance = allowance.saturating_add(n);
    }
    moved
}
