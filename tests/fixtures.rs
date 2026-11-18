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

fn directory_case(case: &Path, rules: &[Rule], reject: bool) {
    let tree = case.join("tree");
    let expect = case.join("expect");
    let mut required = BTreeSet::from([tree.clone()]);
    if reject {
        required.insert(expect.clone());
    }
    assert_eq!(entries(case), required, "{}", case.display());
    let mut files = Vec::new();
    collect_sources(&tree, &tree, &mut files);
    assert!(!files.is_empty(), "empty {}", tree.display());
    let hits = run(rules, files.iter().map(|(p, b)| (p.as_str(), b.as_slice())));
    if reject {
        assert!(!hits.is_empty(), "no finding for {}", case.display());
        let mut actual = String::new();
        for hit in hits {
            writeln!(actual, "{}:{}:{}", hit.path, hit.line, hit.col).unwrap();
        }
        if std::env::var("BLACKHOLE_BLESS").as_deref() == Ok("1") {
            fs::write(expect, actual).unwrap();
        } else {
            assert_eq!(
                fs::read_to_string(expect).unwrap(),
                actual,
                "{}",
                case.display()
            );
        }
    } else {
        assert!(hits.is_empty(), "{}", case.display());
    }
}

fn verify(rule: &Rule, dir: &Path) {
    assert!(!rule.fix.trim().is_empty() && !rule.provenance.trim().is_empty());
    let accept = dir.join("accept");
    let reject = dir.join("reject");
    assert_eq!(
        entries(dir),
        BTreeSet::from([accept.clone(), reject.clone()])
    );
    let clean = entries(&accept);
    assert!(!clean.is_empty());
    for file in clean {
        if file.is_dir() {
            directory_case(&file, registry(), false);
            continue;
        }
        let data = fs::read(&file).unwrap();
        assert!(
            run(registry(), [(file.to_str().unwrap(), data.as_slice())]).is_empty(),
            "{}",
            file.display()
        );
    }
    let rejects = entries(&reject);
    assert!(!rejects.is_empty());
    for file in rejects {
        if file.is_dir() {
            directory_case(&file, std::slice::from_ref(rule), true);
            continue;
        }
        let name = file.file_name().unwrap().to_str().unwrap();
        if let Some(base) = name.strip_suffix(".expect") {
            assert!(reject.join(base).is_file(), "orphan {}", file.display());
            continue;
        }
        let bytes = fs::read(&file).unwrap();
        let hits = run(std::slice::from_ref(rule), [(name, bytes.as_slice())]);
        assert!(!hits.is_empty(), "no finding for {}", file.display());
        let mut actual = String::new();
        for hit in hits {
            writeln!(actual, "{}:{}", hit.line, hit.col).unwrap();
        }
        let expected = reject.join(format!("{name}.expect"));
        if std::env::var("BLACKHOLE_BLESS").as_deref() == Ok("1") {
            fs::write(expected, actual).unwrap();
        } else {
            assert_eq!(fs::read_to_string(expected).unwrap(), actual, "{name}");
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
