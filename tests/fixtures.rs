use blackhole_check::engine::{Rule, registry, run};
use std::{
    collections::BTreeSet,
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
        let Some(expected) = &self.expect else {
            assert!(hits.is_empty(), "{}", path.display());
            return;
        };
        assert!(!hits.is_empty(), "no finding for {}", path.display());
        let mut actual = String::new();
        for hit in hits {
            if self.directory {
                write!(actual, "{}:", hit.path).unwrap();
            }
            writeln!(actual, "{}:{}", hit.line, hit.col).unwrap();
        }
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
            Case::load(&path, side).verify(&path, rule);
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
