use blackhole_check::engine::{Rule, registry, run};
use std::{
    collections::BTreeSet,
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

fn entries(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn verify(rule: &Rule, dir: &Path) {
    assert!(!rule.fix.trim().is_empty() && !rule.provenance.trim().is_empty());
    let accept = dir.join("accept");
    let reject = dir.join("reject");
    assert_eq!(
        entries(dir).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([accept.clone(), reject.clone()])
    );
    let clean = entries(&accept);
    assert_ne!(clean, [] as [PathBuf; 0]);
    for file in clean {
        let data = fs::read(&file).unwrap();
        assert!(
            run(registry(), [(file.to_str().unwrap(), data.as_slice())]).is_empty(),
            "{}",
            file.display()
        );
    }
    let mut count = 0;
    for file in entries(&reject) {
        let name = file.file_name().unwrap().to_str().unwrap();
        if let Some(base) = name.strip_suffix(".expect") {
            assert!(reject.join(base).is_file(), "orphan {}", file.display());
            continue;
        }
        count += 1;
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
    assert!(count > 0);
}

#[test]
fn fixtures() {
    let root = Path::new("rules");
    let expected: BTreeSet<_> = registry().iter().map(|r| root.join(r.id)).collect();
    let families: BTreeSet<_> = expected
        .iter()
        .map(|p| p.parent().unwrap().to_owned())
        .collect();
    assert_eq!(entries(root).into_iter().collect::<BTreeSet<_>>(), families);
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
