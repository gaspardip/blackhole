use crate::{
    engine::Finding,
    files::LEDGER_PATH,
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
pub enum Kind {
    Added,
    Deleted,
    Edited,
    Moved(String),
}
pub struct Change {
    pub kind: Kind,
    pub path: String,
}
impl Change {
    fn old(&self) -> Option<&str> {
        match &self.kind {
            Kind::Moved(old) => Some(old),
            _ => None,
        }
    }
}
pub(crate) fn renames(changes: &[Change]) -> BTreeMap<&str, &str> {
    changes
        .iter()
        .filter_map(|c| Some((c.old()?, c.path.as_str())))
        .collect()
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
    pub test_removals: &'a [Finding],
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
    pub(crate) fn unreadable(error: &io::Error) -> Self {
        Self {
            gates: BTreeSet::from(["unreadable"]),
            lines: format!("{error}\n").into_bytes(),
        }
    }
}

fn protected(i: &Inputs<'_>, moved: &Counts, guard: &GlobSet, v: &mut Verdict) -> io::Result<()> {
    let judged = |path: &str| !i.excludes.is_match(path);
    // Git reports a move it does not recognise as a deletion and an addition.
    let dropped = i
        .changes
        .iter()
        .any(|c| matches!(c.kind, Kind::Deleted) && judged(&c.path));
    let mut paths = BTreeSet::new();
    for c in i.changes {
        // shortcut: a file emptied in place, or moved over two pull requests, still leaves unseen; protect every addition under an excluded path when a repository can afford that approval
        let left = match &c.kind {
            // Git may pair the arrival with another excluded file and leave the judged one as a deletion.
            Kind::Moved(old) => judged(old) || dropped,
            Kind::Added => dropped,
            Kind::Deleted | Kind::Edited => false,
        };
        if left && !judged(&c.path) {
            paths.insert(c.path.as_str());
        }
        for path in c.old().into_iter().chain([c.path.as_str()]) {
            let guarded = if path == LEDGER_PATH {
                !i.head_ledger
                    .is_ok_and(|h| ledger::excess(h, moved).is_empty())
            } else {
                crate::judge_files::protected(path) || guard.is_match(path)
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
            "protected: {path}: a person has to approve this change"
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
    judge_with_guard(i, &GlobSet::empty(), &renames(i.changes))
}

pub(crate) fn judge_with_guard(
    i: &Inputs<'_>,
    guard: &GlobSet,
    renames: &BTreeMap<&str, &str>,
) -> io::Result<Verdict> {
    let mut v = Verdict::default();
    let empty = Counts::new();
    let head = i.head_ledger.unwrap_or(&empty);
    let mut allowed = head.clone();
    let mut removal_lines = Vec::new();
    if !i.approved {
        let moved = ledger::moved_by(i.base_ledger, renames);
        protected(i, &moved, guard, &mut v)?;
        if !i.test_removals.is_empty() {
            v.gates.insert("tests");
            crate::print_findings(i.test_removals, &mut removal_lines)?;
        }
        // A count that grew without an approval would otherwise excuse its own findings.
        for (key, n) in &mut allowed {
            *n = (*n).min(moved.get(key).copied().unwrap_or(0));
        }
    }
    let rows = eligible(i, &mut v)?;
    if let Err(e) = i.head_ledger {
        v.gates.insert("ledger");
        writeln!(v.lines, "{e}")?;
    }
    let actual = ledger::counts(i.findings);
    let stale = ledger::excess(head, &actual);
    if !stale.is_empty() {
        v.gates.insert("ledger");
        crate::report_stale(&stale, &actual, &mut v.lines)?;
    }
    let over = ledger::excess(&actual, &objections::allowances(&allowed, &rows, i.today));
    if !over.is_empty() {
        v.gates.insert("findings");
    }
    crate::report(i.findings, &over, &mut v.lines)?;
    v.lines.append(&mut removal_lines);
    Ok(v)
}
