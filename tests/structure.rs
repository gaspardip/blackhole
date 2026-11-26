mod common;
use std::fs;

#[test]
fn naming_uses_the_whole_tree_and_reports_selected_paths() -> common::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    fs::create_dir_all(root.join("tests/pkg"))?;
    for (name, contents) in [
        ("tests/pkg/package.json", "{}"),
        ("tests/pkg/good.ts", ""),
        ("first.specs.ts", ""),
        ("second.mock.ts", ""),
    ] {
        fs::write(root.join(name), contents)?;
    }
    common::git(root, &["init", "-q"])?;
    common::git(root, &["add", "."])?;
    common::git(root, &["commit", "-qm", "case"])?;
    for args in [
        vec!["check", "first.specs.ts", "tests/pkg/good.ts"],
        vec!["ci", "--base", "HEAD", "--head", "HEAD"],
    ] {
        let output = common::command(env!("CARGO_BIN_EXE_blackhole"))
            .args(&args)
            .current_dir(root)
            .output()?;
        let report = String::from_utf8(output.stdout)?;
        assert_eq!(output.status.code(), Some(1), "{report}");
        assert!(report.contains("first.specs.ts:1:1 ts/test-file-names"));
        assert_eq!(
            report.contains("second.mock.ts:1:1 ts/test-file-names"),
            args[0] == "ci"
        );
        assert!(!report.contains("good.ts"));
    }
    Ok(())
}

#[test]
fn naming_survives_parse_failure() {
    use blackhole_check::engine::{registry, run};
    let rule = registry()
        .iter()
        .find(|rule| rule.id == "ts/test-file-names")
        .expect("file naming rule is registered");
    let hits = run(
        std::slice::from_ref(rule),
        [("x.specs.ts", b"const = ;".as_slice())],
    );
    assert_eq!(
        hits.iter()
            .map(|h| (h.path.as_str(), h.line, h.col))
            .collect::<Vec<_>>(),
        [("x.specs.ts", 1, 1)]
    );
}

#[test]
fn exports_reports_invalid_manifests_and_root_wildcards() {
    use blackhole_check::engine::{registry, run};
    let hits = run(
        registry(),
        [
            ("nested/package.json", b"{" as &[u8]),
            (
                "package.json",
                br#"{"exports":[null,{"import":"./src/../src/*.mts"},"./*.js","./src/../src/*.mts"]}"#,
            ),
        ],
    );
    let locations: Vec<_> = hits
        .iter()
        .map(|h| (h.rule, h.path.as_str(), h.line, h.col))
        .collect();
    assert_eq!(
        locations,
        [
            ("ts/package-exports", "nested/package.json", 1, 1),
            ("ts/package-exports", "package.json", 1, 1)
        ]
    );
    assert_eq!(
        hits[1].note.as_deref(),
        Some("unrestricted source exports: ./*.js, ./src/../src/*.mts")
    );
}

#[test]
fn exports_reads_the_selected_tree() -> common::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let manifest = root.join("package.json");
    fs::write(&manifest, r#"{"exports":"./src/*.ts"}"#)?;
    common::git(root, &["init", "-q"])?;
    common::git(root, &["add", "."])?;
    common::git(root, &["commit", "-qm", "case"])?;
    for (args, red) in [
        (vec!["check", "package.json"], true),
        (vec!["ci", "--base", "HEAD", "--head", "HEAD"], true),
        (vec!["check"], false),
    ] {
        let output = common::command(env!("CARGO_BIN_EXE_blackhole"))
            .args(args)
            .current_dir(root)
            .output()?;
        let report = String::from_utf8(output.stdout)?;
        assert_eq!(output.status.code(), Some(i32::from(red)), "{report}");
        assert_eq!(
            report.contains("package.json:1:1 ts/package-exports"),
            red,
            "{report}"
        );
        fs::write(&manifest, "{}")?;
    }
    Ok(())
}
