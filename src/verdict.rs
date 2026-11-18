use crate::{
    engine::Finding,
    ledger::{self, Counts},
    objections::{self, Date, Rows},
};
use globset::GlobSet;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approvals {
    authors: Vec<String>,
    reviews: Vec<Review>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    login: String,
    state: String,
    commit: String,
    permission: String,
}
impl Approvals {
    #[must_use]
    pub fn approved(&self, head: &str) -> bool {
        let mut last = BTreeMap::new();
        for r in &self.reviews {
            let wrote = self
                .authors
                .iter()
                .any(|a| a.eq_ignore_ascii_case(&r.login));
            if !wrote && r.state != "COMMENTED" {
                last.insert(r.login.to_ascii_lowercase(), r);
            }
        }
        last.values().any(|r| {
            r.state == "APPROVED"
                && r.commit == head
                && matches!(r.permission.as_str(), "write" | "admin" | "maintain")
        })
    }
}
pub struct Change {
    pub old: Option<String>,
    pub path: String,
}
pub struct Inputs<'a> {
    pub approved: bool,
    pub today: Date,
    pub excludes: &'a GlobSet,
    pub changes: &'a [Change],
    pub base_ledger: &'a Counts,
    pub head_ledger: Result<&'a Counts, &'a io::Error>,
    pub base_rows: &'a Rows,
    pub head_rows: Result<&'a Rows, &'a io::Error>,
    pub findings: &'a [Finding],
}
#[derive(Default)]
pub struct Verdict {
    pub gates: BTreeSet<&'static str>,
    pub lines: Vec<u8>,
}
impl Verdict {
    pub(crate) fn behind() -> Self {
        // shortcut: judging the merge result instead needs git merge-tree, do it when requiring up-to-date branches hurts
        Self {
            gates: BTreeSet::from(["behind"]),
            lines: b"head does not contain base; update the branch\n".to_vec(),
        }
    }
}

fn protected(i: &Inputs<'_>, v: &mut Verdict) -> io::Result<()> {
    if i.approved {
        return Ok(());
    }
    let renames: Vec<_> = i
        .changes
        .iter()
        .filter_map(|c| Some((c.old.as_deref()?, c.path.as_str())))
        .collect();
    let mut paths = BTreeSet::new();
    for c in i.changes {
        if c.old
            .as_ref()
            .is_some_and(|p| !i.excludes.is_match(p) && i.excludes.is_match(&c.path))
        {
            paths.insert(c.path.as_str());
        }
        for path in c.old.iter().chain([&c.path]).map(String::as_str) {
            let guarded = match path {
                ".blackhole/ledger" => !i
                    .head_ledger
                    .is_ok_and(|h| ledger::growth(i.base_ledger, h, &renames).is_empty()),
                ".blackhole/objections" => !i
                    .head_rows
                    .is_ok_and(|h| h.iter().all(|(k, r)| i.base_rows.get(k) == Some(r))),
                _ => {
                    path == "blackhole.toml"
                        || path.starts_with(".github/")
                        || path.starts_with(".blackhole/")
                }
            };
            if guarded {
                paths.insert(path);
            }
        }
    }
    for path in paths {
        v.gates.insert("protected");
        writeln!(
            v.lines,
            "protected: {path}: needs an approving review from a person with write access who wrote none of its commits"
        )?;
    }
    Ok(())
}

fn eligible(i: &Inputs<'_>, v: &mut Verdict) -> io::Result<Rows> {
    let mut rows = Rows::new();
    match i.head_rows {
        Err(e) => {
            v.gates.insert("objections");
            write!(v.lines, "objection: ")?;
            if let Some((rule, path)) = e
                .get_ref()
                .and_then(|e| e.downcast_ref::<objections::RowError>())
                .and_then(|e| e.key.as_ref())
            {
                write!(v.lines, "{rule} {path}: ")?;
            }
            writeln!(v.lines, "{e}")?;
        }
        Ok(head) => {
            for (key, r) in head {
                let prev = i.base_rows.get(key);
                let valid = prev.map_or(r.renewal.is_none(), |p| {
                    p == r
                        || (p.renewal.is_none()
                            && r.renewal.is_some()
                            && p.extra == r.extra
                            && p.filed == r.filed
                            && p.reason == r.reason)
                });
                if !valid {
                    v.gates.insert("objections");
                    writeln!(
                        v.lines,
                        "objection: {} {}: invalid transition",
                        key.0, key.1
                    )?;
                } else if i.approved || prev == Some(r) {
                    rows.insert(key.clone(), r.clone());
                }
            }
        }
    }
    Ok(rows)
}

/// # Errors
/// Propagates diagnostic formatting errors.
pub fn judge(i: &Inputs<'_>) -> io::Result<Verdict> {
    let mut v = Verdict::default();
    protected(i, &mut v)?;
    let rows = eligible(i, &mut v)?;
    let empty = Counts::new();
    let head = i.head_ledger.unwrap_or(&empty);
    if let Err(e) = i.head_ledger {
        v.gates.insert("ledger");
        writeln!(v.lines, "{e}")?;
    }
    // An exact head ledger that did not grow keeps every ledger count within the base allowance.
    let actual = ledger::counts(i.findings);
    let stale = ledger::excess(head, &actual);
    if !stale.is_empty() {
        v.gates.insert("ledger");
        crate::report_stale(&stale, &actual, &mut v.lines)?;
    }
    let over = ledger::excess(&actual, &objections::allowances(head, &rows, i.today));
    if !over.is_empty() {
        v.gates.insert("findings");
        crate::report(i.findings, &over, &mut v.lines)?;
    }
    Ok(v)
}
