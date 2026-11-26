mod common;
mod trees;

use blackhole_check::{
    ci,
    engine::{Rule, Run, registry, run},
};
use std::{
    collections::BTreeSet,
    env,
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

fn entries(path: &Path) -> BTreeSet<PathBuf> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn collect_sources(root: &Path, dir: &Path, files: &mut Vec<(String, Vec<u8>)>) {
    for path in entries(dir) {
        if path.is_dir() {
            collect_sources(root, &path, files);
        } else {
            let name = path.strip_prefix(root).unwrap().to_str().unwrap();
            files.push((name.to_owned(), fs::read(path).unwrap()));
        }
    }
}

struct Case {
    files: Vec<(String, Vec<u8>)>,
    expect: Option<PathBuf>,
    directory: bool,
}

impl Case {
    fn load(path: &Path, side: &str) -> Self {
        let directory = path.is_dir();
        let reject = side == "reject";
        let mut files = Vec::new();
        let expect = if directory {
            let tree = path.join("tree");
            let expect = reject.then(|| path.join("expect"));
            let mut required = BTreeSet::from([tree.clone()]);
            required.extend(expect.iter().cloned());
            assert_eq!(entries(path), required, "{}", path.display());
            collect_sources(&tree, &tree, &mut files);
            assert!(!files.is_empty(), "empty {}", tree.display());
            expect
        } else {
            let name = path.file_name().unwrap().to_str().unwrap();
            let logical = if reject { name } else { path.to_str().unwrap() };
            files.push((logical.to_owned(), fs::read(path).unwrap()));
            reject.then(|| path.with_file_name(format!("{name}.expect")))
        };
        Self {
            files,
            expect,
            directory,
        }
    }

    fn verify(&self, path: &Path, rule: &Rule) {
        let rules = if self.expect.is_some() {
            std::slice::from_ref(rule)
        } else {
            registry()
        };
        let hits = run(
            rules,
            self.files.iter().map(|(p, b)| (p.as_str(), b.as_slice())),
        );
        verify_locations(
            self.expect.as_deref(),
            path,
            hits.into_iter().map(|hit| {
                let prefix = if self.directory {
                    hit.path
                } else {
                    String::new()
                };
                (prefix, hit.line, hit.col)
            }),
        );
    }
}

fn verify_locations(
    expected: Option<&Path>,
    path: &Path,
    hits: impl IntoIterator<Item = (String, usize, usize)>,
) {
    let mut actual = String::new();
    for (name, line, col) in hits {
        if !name.is_empty() {
            write!(actual, "{name}:").unwrap();
        }
        writeln!(actual, "{line}:{col}").unwrap();
    }
    let Some(expected) = expected else {
        assert!(actual.is_empty(), "{}: {actual}", path.display());
        return;
    };
    assert!(!actual.is_empty(), "no finding for {}", path.display());
    if std::env::var("BLACKHOLE_BLESS").as_deref() == Ok("1") {
        fs::write(expected, actual).unwrap();
    } else {
        assert_eq!(
            fs::read_to_string(expected).unwrap(),
            actual,
            "{}",
            path.display()
        );
    }
}

fn diff_case(path: &Path, side: &str, rule: &Rule) -> common::Result<()> {
    let expected = (side == "reject").then(|| path.join("expect"));
    let mut required = BTreeSet::from([path.join("base"), path.join("head")]);
    required.extend(expected.iter().cloned());
    assert_eq!(entries(path), required, "{}", path.display());

    let temp = tempfile::tempdir()?;
    let (base, head) = trees::revisions(path, temp.path())?;
    let original = env::current_dir()?;
    env::set_current_dir(temp.path())?;
    let findings = ci::diff_findings(&base, &head);
    env::set_current_dir(original)?;
    let hits: Vec<_> = findings?
        .into_iter()
        .filter(|hit| hit.rule == rule.id)
        .collect();
    if path.ends_with("rename-fewer-cases") {
        assert_eq!(
            hits.first().and_then(|hit| hit.note.as_deref()),
            Some("test cases 3 -> 2, assertions 3 -> 2")
        );
    }
    verify_locations(
        expected.as_deref(),
        path,
        hits.into_iter().map(|hit| (hit.path, hit.line, hit.col)),
    );
    Ok(())
}

fn verify(rule: &Rule, dir: &Path) {
    assert!(!rule.fix.trim().is_empty() && !rule.provenance.trim().is_empty());
    assert_eq!(
        entries(dir),
        BTreeSet::from([dir.join("accept"), dir.join("reject")])
    );
    for side in ["accept", "reject"] {
        let folder = dir.join(side);
        let cases = entries(&folder);
        assert!(!cases.is_empty(), "empty {}", folder.display());
        for path in cases {
            if side == "reject"
                && !path.is_dir()
                && let Some(name) = path
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .strip_suffix(".expect")
            {
                assert!(folder.join(name).is_file(), "orphan {}", path.display());
                continue;
            }
            if matches!(rule.run, Run::Diff(_)) {
                diff_case(&path, side, rule).unwrap();
            } else {
                Case::load(&path, side).verify(&path, rule);
            }
        }
    }
}

#[test]
fn fixtures() {
    let root = Path::new("rules");
    let expected: BTreeSet<_> = registry().iter().map(|r| root.join(r.id)).collect();
    let families: BTreeSet<_> = expected
        .iter()
        .map(|p| p.parent().unwrap().to_owned())
        .collect();
    assert_eq!(entries(root), families);
    assert_eq!(
        families
            .iter()
            .flat_map(|p| entries(p))
            .collect::<BTreeSet<_>>(),
        expected
    );
    for rule in registry() {
        verify(rule, &root.join(rule.id));
    }
}
