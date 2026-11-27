#[path = "common/agent.rs"]
mod agent;
#[path = "common/cli.rs"]
mod cli;
pub mod common;
use agent::run;
use cli::repo;
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::tempdir;

const EDIT_ERRORS: [(&str, i32); 2] = [("pre-edit", 2), ("post-edit", 1)];

fn hook(root: &Path, event: &str, payload: &Value, status: i32) -> Vec<u8> {
    let out = run(root, &["hook", event], &payload.to_string(), status, false);
    assert_eq!(out.stdout, b"");
    out.stderr
}

fn check(root: &Path, paths: &[&str], status: i32) -> Vec<u8> {
    cli::check(root, paths, status).stdout
}

fn patch(text: &str) -> Value {
    json!({"tool_name":"apply_patch","tool_input":{"command":text}})
}

fn edit(tool: &str, path: &str) -> Value {
    let key = if tool == "NotebookEdit" {
        "notebook_path"
    } else {
        "file_path"
    };
    json!({"tool_name":tool,"tool_input":{key:path}})
}

#[test]
fn edits_and_stop_share_check_output() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.py"), "# noqa").unwrap();
    let _ = run(root, &["init"], "", 0, false);
    assert_eq!(check(root, &[], 0), b"");
    assert_eq!(hook(root, "post-edit", &edit("Write", "a.py"), 0), b"");
    assert_eq!(hook(root, "stop", &json!({}), 0), b"");
    fs::write(root.join("a.py"), "# noqa\n# noqa").unwrap();
    fs::write(root.join("b.py"), "# noqa").unwrap();
    let one = check(root, &["a.py"], 1);
    for tool in ["Edit", "MultiEdit", "Write", "NotebookEdit"] {
        assert_eq!(hook(root, "post-edit", &edit(tool, "a.py"), 2), one);
    }
    let patch =
        patch("*** Begin Patch\n*** Delete File: a.py\n*** Add File: b.py\n+x\n*** End Patch");
    let all = check(root, &[], 1);
    assert_eq!(hook(root, "post-edit", &patch, 2), all);
    for active in [true, false] {
        let stop = hook(root, "stop", &json!({"stop_hook_active":active}), 2);
        assert!(stop.starts_with(&all));
        assert!(
            String::from_utf8(stop)
                .unwrap()
                .lines()
                .last()
                .unwrap()
                .contains("blackhole object")
        );
    }
}

fn ledger_repo() -> tempfile::TempDir {
    let dir = repo();
    fs::create_dir(dir.path().join(".blackhole")).unwrap();
    fs::write(dir.path().join(".blackhole/ledger"), "blackhole-ledger 1\n").unwrap();
    dir
}

fn refuses(root: &Path, path: &Path) {
    refuses_from(root, None, path);
}

fn refuses_from(root: &Path, cwd: Option<&Path>, path: &Path) {
    let mut payload = edit("Write", path.to_str().unwrap());
    if let Some(cwd) = cwd {
        payload["cwd"] = json!(cwd);
    }
    let msg = String::from_utf8(hook(root, "pre-edit", &payload, 2)).unwrap();
    assert!(
        msg.contains("blackhole ledger") && msg.contains("blackhole object"),
        "{} from {cwd:?}",
        path.display()
    );
}

#[test]
fn a_direct_write_cannot_create_the_ledger() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".blackhole")).unwrap();
    refuses(root, Path::new(".blackhole/ledger"));
    assert!(!root.join(".blackhole/ledger").exists());
}

#[test]
fn ledger_case_variants_are_refused_on_every_filesystem() {
    let dir = ledger_repo();
    refuses(dir.path(), &dir.path().join(".BLACKHOLE/LEDGER"));
}

#[test]
fn ledger_move_destination() {
    let dir = ledger_repo();
    let root = dir.path();
    let patch = patch(
        "*** Begin Patch\n*** Update File: ordinary\n*** Move to: .blackhole/ledger\n@@\n-a\n+b\n*** End Patch",
    );
    hook(root, "pre-edit", &patch, 2);
    hook(root, "pre-edit", &edit("Write", "ordinary"), 0);
}

#[test]
fn malformed_and_unrelated_inputs() {
    let dir = repo();
    let root = dir.path();
    for (event, code) in EDIT_ERRORS {
        for bad in ["", "{"] {
            assert_ne!(run(root, &["hook", event], bad, code, false).stderr, b"");
        }
        hook(
            root,
            event,
            &json!({"tool_name":"Edit","tool_input":{}}),
            code,
        );
        hook(root, event, &json!({"tool_name":"Bash"}), 0);
    }
    fs::write(root.join("blackhole.toml"), "exclude=['excluded.py']").unwrap();
    fs::write(root.join("excluded.py"), "# noqa").unwrap();
    for path in ["../absent", "gone.py", "excluded.py"] {
        hook(root, "post-edit", &edit("Write", path), 0);
    }
}

#[cfg(unix)]
#[test]
fn unreadable_target_is_refused_before_edit() {
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink("loop", root.join("loop")).unwrap();
    let msg = hook(root, "pre-edit", &edit("Write", "loop/file"), 2);
    assert!(String::from_utf8(msg).unwrap().contains("target"));
}

#[cfg(unix)]
#[test]
fn post_edit_agrees_with_check_for_a_listed_external_link() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir(&root).unwrap();
    common::git(&root, &["init", "-q"]).unwrap();
    fs::write(dir.path().join("outside.py"), "# noqa").unwrap();
    std::os::unix::fs::symlink("../outside.py", root.join("alias.py")).unwrap();
    let expected = check(&root, &["alias.py"], 1);
    for payload in tool_edits("alias.py") {
        assert_eq!(hook(&root, "pre-edit", &payload, 0), b"");
        assert_eq!(hook(&root, "post-edit", &payload, 2), expected);
    }
    let _ = run(&root, &["init"], "", 0, false);
    fs::write(root.join("clean.py"), "pass").unwrap();
    fs::write(dir.path().join("outside.py"), "pass").unwrap();
    assert_eq!(
        hook(
            &root,
            "post-edit",
            &patch(
                "*** Begin Patch\n*** Delete File: alias.py\n*** Add File: clean.py\n*** End Patch"
            ),
            2
        ),
        check(&root, &["alias.py", "clean.py"], 1)
    );
}

#[cfg(unix)]
#[test]
fn absent_ledger_cannot_be_created_through_dangling_links() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".blackhole")).unwrap();
    std::os::unix::fs::symlink(".blackhole/ledger", root.join("alias")).unwrap();
    std::os::unix::fs::symlink("alias", root.join("second")).unwrap();
    refuses(root, Path::new("alias"));
    refuses(root, Path::new("second"));
}

#[test]
fn absent_ledger_cannot_be_created_through_a_normalized_path() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".blackhole")).unwrap();
    refuses(root, Path::new("missing/../.blackhole/ledger"));
}

#[test]
fn absent_ledger_cannot_be_created_through_a_case_variant() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".blackhole")).unwrap();
    for path in [
        ".blackhole/LEDGER",
        ".BlackHole/Ledger",
        ".blac\u{212a}hole/ledger",
    ] {
        refuses(root, Path::new(path));
    }
}

#[cfg(unix)]
#[test]
fn resolved_directory_spellings_cannot_create_the_ledger() {
    use std::os::unix::fs::symlink;
    let dir = repo();
    let root = dir.path();
    fs::create_dir_all(root.join("real/nested")).unwrap();
    symlink("real", root.join(".blackhole")).unwrap();
    symlink("real/nested", root.join("alias")).unwrap();
    for path in [
        "alias/../ledger",
        "alias/../.blackhole/ledger",
        "real/ledger",
        ".blackhole/ledger",
    ] {
        refuses(root, Path::new(path));
    }
    assert!(!root.join("real/ledger").exists());
}

#[cfg(unix)]
#[test]
fn post_edit_matches_check_with_an_unreadable_ignored_directory() {
    use std::os::unix::fs::PermissionsExt;
    let dir = repo();
    let root = dir.path();
    let ignored = root.join("ignored");
    fs::write(root.join(".gitignore"), "/ignored/\n").unwrap();
    fs::write(root.join("red.py"), "# noqa").unwrap();
    fs::create_dir(&ignored).unwrap();
    fs::set_permissions(&ignored, fs::Permissions::from_mode(0o000)).unwrap();
    let expected = check(root, &[], 1);
    assert!(expected.starts_with(b"red.py:1:1 "));
    let actual = hook(root, "post-edit", &edit("Write", "red.py"), 2);
    fs::set_permissions(&ignored, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn stop_blocks_when_the_checkout_check_cannot_complete() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("blackhole.toml"), "broken = [").unwrap();
    let error = String::from_utf8(hook(root, "stop", &json!({}), 2)).unwrap();
    assert!(error.starts_with("blackhole: blackhole.toml:"));
    assert_eq!(error.lines().count(), 1);
}

#[test]
fn blocking_hooks_block_when_git_reports_the_wrong_checkout_root() {
    let dir = repo();
    let root = dir.path();
    let outside = tempdir().unwrap();
    common::git(
        root,
        &["config", "core.worktree", outside.path().to_str().unwrap()],
    )
    .unwrap();
    for (event, payload) in [("stop", json!({})), ("pre-edit", edit("Write", "a.py"))] {
        let error = String::from_utf8(hook(root, event, &payload, 2)).unwrap();
        assert!(error.contains("checkout root mismatch"), "{event}: {error}");
        assert_ne!(hook(outside.path(), event, &payload, 1), b"");
    }
}

#[cfg(unix)]
#[test]
fn check_resolution_errors_escape_the_path() {
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink("loop\n", root.join("loop\n")).unwrap();
    let out = run(root, &["check", "loop\n/file.py"], "", 2, false);
    assert_eq!(out.stdout, b"");
    let error = String::from_utf8(out.stderr).unwrap();
    assert!(error.contains(r"loop\n/file.py"), "{error:?}");
    assert_eq!(error.lines().count(), 1);
}

#[test]
fn ignored_tools_outside_git_are_silent() {
    let dir = tempdir().unwrap();
    for event in ["pre-edit", "post-edit"] {
        assert_eq!(
            hook(dir.path(), event, &json!({"tool_name":"Bash"}), 0),
            b""
        );
    }
}

#[test]
fn post_edit_reports_stale_selected_allowance() {
    let dir = repo();
    let root = dir.path();
    for name in ["a.py", "b.py"] {
        fs::write(root.join(name), "# noqa").unwrap();
    }
    let _ = run(root, &["init"], "", 0, false);
    for name in ["a.py", "b.py"] {
        fs::write(root.join(name), "pass").unwrap();
    }
    let expected = check(root, &["a.py"], 1);
    assert_eq!(
        expected,
        b"a.py core/no-suppression: ledger allows 1, found 0; run blackhole ledger\n"
    );
    assert_eq!(hook(root, "post-edit", &edit("Edit", "a.py"), 2), expected);
}

#[test]
fn invalid_target_names_report_one_line() {
    let dir = repo();
    let root = dir.path();
    for path in [
        "bad\0name".to_owned(),
        "x".repeat(4097),
        "a/".repeat(60_000),
    ] {
        for (event, status) in EDIT_ERRORS {
            let message = hook(root, event, &edit("Write", &path), status);
            assert_eq!(String::from_utf8(message).unwrap().lines().count(), 1);
        }
    }
}

#[test]
fn finding_path_is_escaped() {
    let dir = repo();
    let root = dir.path();
    for (name, escaped) in [
        ("line\nbreak.py", r"line\nbreak.py:1:1 "),
        (
            "é\u{85}\u{2028}\u{2029}.py",
            r"é\u{85}\u{2028}\u{2029}.py:1:1 ",
        ),
    ] {
        fs::write(root.join(name), "# noqa").unwrap();
        for event in ["post-edit", "stop"] {
            let text = String::from_utf8(hook(root, event, &edit("Write", name), 2)).unwrap();
            assert!(text.starts_with(escaped), "{text:?}");
            assert_eq!(text.lines().count(), if event == "stop" { 3 } else { 2 });
        }
        fs::remove_file(root.join(name)).unwrap();
    }
}

#[test]
fn payload_one_byte_over_limit_is_refused() {
    let dir = repo();
    let root = dir.path();
    let mut payload = String::from(r#"{"tool_name":"Bash"}"#);
    payload.extend(std::iter::repeat_n(
        ' ',
        64 * 1024 * 1024 + 1 - payload.len(),
    ));
    for (event, status) in EDIT_ERRORS {
        let out = run(root, &["hook", event], &payload, status, true);
        assert!(String::from_utf8(out.stderr).unwrap().contains("64 MiB"));
    }
    for status in [0, 2] {
        if status == 2 {
            fs::write(root.join("red.py"), "# noqa").unwrap();
        }
        let expected = check(root, &[], i32::from(status == 2));
        for (data, partial) in [("", false), ("{", false), (payload.as_str(), true)] {
            let out = run(root, &["hook", "stop"], data, status, partial);
            assert!(out.stderr.starts_with(&expected));
            assert_eq!(out.stderr.is_empty(), status == 0);
        }
    }
}

#[test]
fn codex_patch_boundaries_and_hunks() {
    let dir = repo();
    let root = dir.path();
    for name in ["new.py", "old.py", "moved.py", "context.py"] {
        fs::write(root.join(name), "# noqa").unwrap();
    }
    let check_patch = |text: &str, files: &[&str]| {
        let payload = patch(text);
        assert_eq!(hook(root, "pre-edit", &payload, 0), b"");
        assert_eq!(
            hook(root, "post-edit", &payload, 2),
            check(root, files, 1),
            "{text}"
        );
    };
    for text in [
        "*** Begin Patch\n*** Update File: new.py\n\r\r\n+x\n*** End Patch",
        "*** Begin Patch \n   *** Add File: new.py  \n+hi\n *** End Patch",
        "<<EOF\n*** Begin Patch\n*** Delete File: new.py\n*** End Patch\nEOF",
        "<<'EOF'\n*** Begin Patch\n*** Add File: new.py\n*** End Patch\nEOF",
        "<<\"EOF\"\n*** Begin Patch\n*** Delete File: new.py\n*** End Patch\n  EOF",
        "*** Begin Patch\n*** Environment ID: local\n*** Update File: new.py\n *** Delete File: context.py\n+line\n*** End of File\n\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n@@\n+x\n*** End of File\n@@ next\n+y\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n*** End of File\n+x\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n\n*** End Patch",
        "*** Begin Patch\n*** Delete File: new.py\n*** End Patch",
    ] {
        check_patch(text, &["new.py"]);
    }
    check_patch(
        "*** Begin Patch\n*** Add File: new.py\n+hi\n*** Update File: old.py  \n*** Move to: moved.py  \n@@ def f():\n-old\n+new\n*** End Patch",
        &["new.py", "old.py", "moved.py"],
    );
    check_patch(
        "*** Begin Patch\n*** Update File: new.py\n*** Move to: moved.py\n*** End of File\n+x\n*** End Patch",
        &["new.py", "moved.py"],
    );
    let empty = patch("*** Begin Patch\n*** End Patch");
    for event in ["pre-edit", "post-edit"] {
        assert_eq!(hook(root, event, &empty, 0), b"");
    }
    for text in [
        "*** Begin Patch\n*** Update File: new.py\n*** Move to: moved.py\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n@@\n@@\n+x\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n@@\n*** End of File\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n+x\nno prefix\n*** End Patch",
        "*** Begin Patch\n*** Environment ID:   \n*** Add File: new.py\n*** End Patch",
        "*** Begin Patch\n*** End Patch\n*** Delete File: new.py\n*** End Patch",
        "bad",
        "*** Begin Patch\nbad",
        "<<PATCH\n*** Begin Patch\n*** Add File: new.py\n*** End Patch\nPATCH",
        "*** Begin Patch\nnoise\n*** Add File: new.py\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n@@\n*** End Patch",
        "*** Begin Patch\n*** Add File: new.py\nmissing plus\n*** End Patch",
        "*** Begin Patch\n*** Environment ID: one\n*** Environment ID: two\n*** Add File: new.py\n*** End Patch",
        "*** Begin Patch\n*** Delete File: new.py\n*** Move to: moved.py\n*** End Patch",
        "*** Begin Patch\n*** Update File: new.py\n+line\n*** End of File\n+extra\n*** End Patch",
    ] {
        let payload = patch(text);
        for (event, status) in EDIT_ERRORS {
            let error = hook(root, event, &payload, status);
            assert!(
                String::from_utf8(error).unwrap().contains("patch"),
                "{text}"
            );
        }
    }
}

#[test]
fn pre_edit_refuses_unreadable_and_ambiguous_targets() {
    let dir = ledger_repo();
    for data in [
        r#"{"tool_name":"Write","tool_input":{"file_path":".blackhole/ledger","content":"\ud800"}}"#,
        r#"{"tool_input":{"file_path":".blackhole/ledger"}}"#,
        r#"{"tool_name":"Write","tool_name":"Bash"}"#,
        r#"{"tool_name":"Write","tool_input":{"file_path":".blackhole/ledger","file_path":"other"}}"#,
    ] {
        let err = run(dir.path(), &["hook", "pre-edit"], data, 2, false).stderr;
        assert_eq!(String::from_utf8(err).unwrap().lines().count(), 1);
    }
    let outside = tempdir().unwrap();
    let _ = run(outside.path(), &["hook", "pre-edit"], "{", 1, false);
}

#[test]
fn lexical_and_filesystem_ledger_spellings() {
    let dir = ledger_repo();
    let root = dir.path();
    fs::create_dir(root.join("real")).unwrap();
    let absolute = root.join(".blackhole/ledger");
    for path in [
        Path::new(".blackhole/ledger"),
        Path::new(".blackhole/../.blackhole/ledger"),
        Path::new("missing/../.blackhole/ledger"),
        Path::new("real/../.blackhole/ledger"),
        Path::new("./.blackhole/ledger"),
        Path::new(".blackhole//ledger"),
        Path::new(".blackhole/ledger/"),
        &absolute,
    ] {
        refuses(root, path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(".blackhole/ledger", root.join("file")).unwrap();
        symlink(".blackhole", root.join("dir")).unwrap();
        for path in ["file", "dir/ledger"] {
            refuses(root, Path::new(path));
        }
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        symlink("sub/deep", root.join("x")).unwrap();
        refuses(root, Path::new("x/../.blackhole/ledger"));
        for (cwd, target) in [
            (root.to_owned(), ".blackhole/ledger"),
            (root.join("x"), "../.blackhole/ledger"),
            ("x".into(), "../.blackhole/ledger"),
        ] {
            refuses_from(root, Some(&cwd), Path::new(target));
        }
        // A writer that starts from where the directory really is and normalizes from there.
        let far = tempdir().unwrap();
        fs::create_dir_all(far.path().join("a/b")).unwrap();
        symlink(far.path().join("a/b"), root.join("sub/deep/out")).unwrap();
        refuses_from(
            root,
            Some(&root.join("x")),
            Path::new("out/../../../.blackhole/ledger"),
        );
        fs::create_dir(root.join(".blackhole/nested")).unwrap();
        symlink(".blackhole/nested", root.join("y")).unwrap();
        refuses(root, Path::new("y/../ledger"));
    }
}

#[test]
fn delete_and_move_report_departed_ledger_rows() {
    for moved in [false, true] {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("a.py"), "# noqa").unwrap();
        let _ = run(root, &["init"], "", 0, false);
        let mut args = vec!["check", "a.py"];
        let hunk = if moved {
            fs::rename(root.join("a.py"), root.join("b.py")).unwrap();
            args.push("b.py");
            "*** Update File: a.py\n*** Move to: b.py\n@@\n # noqa"
        } else {
            fs::remove_file(root.join("a.py")).unwrap();
            "*** Delete File: a.py"
        };
        let patch = patch(&format!("*** Begin Patch\n{hunk}\n*** End Patch"));
        let expected = run(root, &args, "", 1, false).stdout;
        assert!(expected.ends_with(b"run blackhole ledger\n"));
        assert_eq!(hook(root, "post-edit", &patch, 2), expected);
    }
}

#[test]
fn hook_output_distinguishes_controls_from_literal_escapes() {
    let dir = ledger_repo();
    let root = dir.path();
    for name in ["é\n.py", r"é\n.py"] {
        fs::write(root.join(name), "# noqa").unwrap();
    }
    let stop = String::from_utf8(hook(root, "stop", &json!({}), 2)).unwrap();
    for name in [r"é\n.py", r"é\\n.py"] {
        assert!(stop.lines().any(|l| l.starts_with(name)), "{stop:?}");
    }
    let name = "é\u{1b}\u{7}\u{7f}\u{85}.py";
    fs::write(root.join(name), "pass").unwrap();
    fs::write(
        root.join(".blackhole/ledger"),
        format!("blackhole-ledger 1\ncore/no-suppression\t1\t{name}\n"),
    )
    .unwrap();
    for event in ["post-edit", "stop"] {
        let msg = String::from_utf8(hook(root, event, &edit("Write", name), 2)).unwrap();
        assert!(
            msg.contains(r"é\u{1b}\u{7}\u{7f}\u{85}.py core/no-suppression"),
            "{msg:?}"
        );
    }
    for (event, status) in EDIT_ERRORS {
        let msg = String::from_utf8(hook(
            root,
            event,
            &edit("Write", "é\nsecond\\part\0"),
            status,
        ))
        .unwrap();
        assert!(msg.contains(r"é\nsecond\\part\0"), "{msg:?}");
        assert_eq!(msg.lines().count(), 1);
    }
}

#[cfg(unix)]
#[test]
fn edits_check_both_path_readings() {
    use std::os::unix::fs::symlink;
    let dir = ledger_repo();
    let root = dir.path();
    fs::create_dir_all(root.join("real/deep")).unwrap();
    for name in ["red.py", "real/red.py", "real/loop/red.py"] {
        let file = root.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "# noqa").unwrap();
    }
    for (target, link) in [
        ("real/deep", "through"),
        ("real", "side"),
        ("red.py", "inside.py"),
        ("red.py", "inside.txt"),
        ("absent.py", "dangling.py"),
        ("loop", "loop"),
    ] {
        symlink(target, root.join(link)).unwrap();
    }
    let outside = tempdir().unwrap();
    let alias = outside.path().join("alias");
    symlink(root, &alias).unwrap();
    let external = outside.path().join("a/b");
    fs::create_dir_all(&external).unwrap();
    symlink(external, root.join("real/deep/out")).unwrap();
    fs::create_dir(root.join(".git/info")).unwrap();
    fs::write(root.join(".git/info/exclude"), "/loop\n").unwrap();
    // Every listed name that reads the written file is reported, so the links to red.py come with it.
    let red = ["inside.py", "inside.txt", "red.py"].as_slice();
    let real = ["real/red.py"].as_slice();
    let both = [red, real].concat();
    let red_path = root.join("red.py");
    for (target, cwd, expected) in [
        ("red.py", root, red),
        ("./red.py", root, red),
        ("real/../red.py", root, red),
        ("missing/../red.py", root, red),
        ("through/../red.py", root, &both),
        (red_path.to_str().unwrap(), root, red),
        ("../red.py", &root.join("real"), red),
        ("../red.py", &root.join("real/deep"), real),
        // The payload's cwd has a textual reading as well as the directory it names.
        ("../red.py", &root.join("through"), &both),
        ("../red.py", Path::new("through"), &both),
        ("../red.py", &alias.join("through"), &both),
        ("red.py", &alias, red),
        ("through/../side/red.py", root, real),
        ("../side/red.py", &root.join("through"), real),
        ("out/../../red.py", &root.join("through"), &both),
        ("through/../loop/red.py", root, &["real/loop/red.py"]),
        ("inside.py", root, red),
        ("inside.txt", root, red),
        ("dangling.py", root, &["dangling.py"]),
    ] {
        let mut args = vec!["check"];
        args.extend_from_slice(expected);
        let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)));
        for mut payload in tool_edits(target) {
            payload["cwd"] = json!(cwd);
            assert_eq!(hook(root, "pre-edit", &payload, 0), b"");
            let status = if out.status.success() { 0 } else { 2 };
            assert_eq!(
                hook(root, "post-edit", &payload, status),
                out.stdout,
                "{target} from {}",
                cwd.display()
            );
        }
    }
}
#[cfg(unix)]
#[test]
fn a_name_git_still_holds_is_checked_through_a_new_link() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join("d")).unwrap();
    fs::write(root.join("d/file.py"), "pass").unwrap();
    common::git(root, &["add", "d/file.py"]).unwrap();
    fs::rename(root.join("d"), root.join("real")).unwrap();
    fs::create_dir(root.join(".git/info")).unwrap();
    fs::write(root.join(".git/info/exclude"), "/real\n").unwrap();
    std::os::unix::fs::symlink("real", root.join("d")).unwrap();
    fs::write(root.join("d/file.py"), "# noqa").unwrap();
    // A second link on the way to the same file is a third name for it.
    std::os::unix::fs::symlink("d", root.join("via")).unwrap();
    let listed = check(root, &["d"], 1);
    assert!(listed.starts_with(b"d/file.py:1:1 "));
    let real = root.join("real/file.py");
    for target in [
        "d/file.py",
        "via/file.py",
        "real/file.py",
        real.to_str().unwrap(),
    ] {
        for payload in tool_edits(target) {
            assert_eq!(hook(root, "post-edit", &payload, 2), listed, "{target}");
        }
    }
}

#[cfg(unix)]
#[test]
fn a_second_name_of_the_written_file_is_reported_with_it() {
    let dir = repo();
    let root = dir.path();
    let outside = tempdir().unwrap();
    let (linked, hard) = (
        outside.path().join("linked.py"),
        outside.path().join("hard.py"),
    );
    fs::write(&linked, "# noqa").unwrap();
    std::os::unix::fs::symlink(&linked, root.join("link.py")).unwrap();
    fs::write(root.join("name.py"), "# noqa").unwrap();
    fs::hard_link(root.join("name.py"), root.join("twin.py")).unwrap();
    fs::hard_link(root.join("name.py"), &hard).unwrap();
    let both = check(root, &["name.py", "twin.py"], 1);
    for target in ["name.py", "twin.py"] {
        assert_eq!(hook(root, "post-edit", &edit("Write", target), 2), both);
    }
    // An edit that lands outside the repository lists nothing, not even a departed file's row: these wait for stop.
    fs::create_dir(root.join(".blackhole")).unwrap();
    fs::write(
        root.join(".blackhole/ledger"),
        "blackhole-ledger 1\ncore/no-suppression\t1\tgone.py\n",
    )
    .unwrap();
    for target in [&linked, &hard] {
        let payload = edit("Write", target.to_str().unwrap());
        assert_eq!(hook(root, "post-edit", &payload, 0), b"");
    }
    let stop = String::from_utf8(hook(root, "stop", &json!({}), 2)).unwrap();
    assert!(stop.contains("gone.py ") && stop.contains("link.py:1:1 "));
}

#[cfg(unix)]
#[test]
fn outside_file_link_landing_in_the_checkout_reports_listed_names() {
    use std::os::unix::fs::symlink;
    let dir = repo();
    let root = dir.path();
    let outside = tempdir().unwrap();
    let back = outside.path().join("back.py");
    fs::write(root.join("red.py"), "# noqa").unwrap();
    fs::write(root.join("unrelated.py"), "# noqa").unwrap();
    symlink(root.join("red.py"), &back).unwrap();
    symlink(&back, root.join("via.py")).unwrap();
    let expected = check(root, &["red.py", "via.py"], 1);
    assert!(expected.starts_with(b"red.py:1:1 "));
    for target in [&back, &root.join("via.py")] {
        for payload in tool_edits(target.to_str().unwrap()) {
            assert_eq!(hook(root, "post-edit", &payload, 2), expected);
        }
    }
}

#[cfg(unix)]
#[test]
fn alternate_directory_reading_does_not_select_files_below_it() {
    use std::os::unix::fs::symlink;
    let dir = ledger_repo();
    let root = dir.path();
    fs::create_dir(root.join("d")).unwrap();
    fs::write(root.join("d/red.py"), "# noqa").unwrap();
    common::git(root, &["add", "d/red.py"]).unwrap();
    fs::rename(root.join("d"), root.join("files")).unwrap();
    symlink("files", root.join("d")).unwrap();
    symlink("files", root.join("alias")).unwrap();
    fs::create_dir_all(root.join("actual/deep")).unwrap();
    fs::write(root.join("actual/alias"), "").unwrap();
    fs::write(root.join("actual/d"), "").unwrap();
    symlink("actual/deep", root.join("through")).unwrap();
    assert!(check(root, &["d"], 1).starts_with(b"d/red.py:1:1 "));
    for name in ["alias", "d"] {
        let target = format!("through/../{name}");
        assert_eq!(hook(root, "post-edit", &edit("Write", &target), 0), b"");
    }
    fs::write(
        root.join(".blackhole/ledger"),
        "blackhole-ledger 1\ncore/no-suppression\t1\td\n",
    )
    .unwrap();
    assert_eq!(
        hook(root, "post-edit", &edit("Write", "through/../alias"), 0),
        b""
    );
}

#[cfg(unix)]
#[test]
fn checkout_alias_preserves_the_departed_ledger_name() {
    use std::os::unix::fs::symlink;
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join("d")).unwrap();
    fs::write(root.join("d/gone.py"), "# noqa").unwrap();
    fs::write(root.join("unrelated.py"), "# noqa").unwrap();
    let _ = run(root, &["init"], "", 0, false);
    fs::rename(root.join("d"), root.join("real")).unwrap();
    symlink("real", root.join("d")).unwrap();
    fs::rename(root.join("real/gone.py"), root.join("moved.py")).unwrap();
    fs::remove_file(root.join("unrelated.py")).unwrap();
    let outside = tempdir().unwrap();
    let alias = outside.path().join("checkout");
    symlink(root, &alias).unwrap();
    let expected = check(root, &["d"], 1);
    assert!(expected.starts_with(b"d/gone.py "));
    for (cwd, target) in [
        (root.to_owned(), alias.join("d/gone.py")),
        (alias.clone(), "d/gone.py".into()),
        (alias.join("d"), "gone.py".into()),
    ] {
        let mut payload = patch(&format!(
            "*** Begin Patch\n*** Delete File: {}\n*** End Patch",
            target.display()
        ));
        payload["cwd"] = json!(cwd);
        assert_eq!(hook(root, "post-edit", &payload, 2), expected);
    }
    let mut moved = patch(
        "*** Begin Patch\n*** Update File: d/gone.py\n*** Move to: moved.py\n@@\n # noqa\n*** End Patch",
    );
    moved["cwd"] = json!(alias);
    assert_eq!(
        hook(root, "post-edit", &moved, 2),
        check(root, &["d", "moved.py"], 1)
    );
}

#[cfg(unix)]
#[test]
fn departed_rows_survive_nested_and_external_directory_aliases() {
    use std::os::unix::fs::symlink;
    let dir = repo();
    let root = dir.path();
    let outside = tempdir().unwrap();
    let view = outside.path().join("view");
    symlink(root, &view).unwrap();
    fs::create_dir_all(root.join("sub/d")).unwrap();
    fs::write(root.join("sub/d/gone.py"), "# noqa").unwrap();
    let _ = run(root, &["init"], "", 0, false);
    fs::rename(root.join("sub/d/gone.py"), root.join("moved.py")).unwrap();
    symlink("sub", root.join("alias")).unwrap();
    symlink("sub/d", root.join("chain")).unwrap();
    symlink("../alias/d", root.join("sub/chain")).unwrap();
    let external = outside.path().join("chain");
    symlink(root.join("sub/d"), &external).unwrap();
    let moved_outside = outside.path().join("moved.py");
    fs::copy(root.join("moved.py"), &moved_outside).unwrap();
    let real = root.join("sub/real");
    fs::rename(root.join("sub/d"), &real).unwrap();
    for destination in [&real, outside.path()] {
        symlink(destination, root.join("sub/d")).unwrap();
        let stale = check(root, &["sub/d"], 1);
        assert!(stale.starts_with(b"sub/d/gone.py "));
        let both = check(root, &["sub/d", "moved.py"], 1);
        let external_file = external.join("gone.py");
        for name in [
            "alias/d/gone.py",
            "sub/d/gone.py",
            "chain/gone.py",
            "sub/chain/gone.py",
            external_file.to_str().unwrap(),
        ] {
            let path = view.join(name);
            let payload = deletes(root.to_str().unwrap(), path.to_str().unwrap(), 1);
            assert_eq!(hook(root, "post-edit", &payload, 2), stale, "{name}");
            for (to, expected) in [
                ("moved.py", &both),
                (moved_outside.to_str().unwrap(), &stale),
            ] {
                let mut moved = patch(&format!(
                    "*** Begin Patch\n*** Update File: {name}\n*** Move to: {to}\n@@\n # noqa\n*** End Patch"
                ));
                moved["cwd"] = json!(view);
                assert_eq!(
                    hook(root, "post-edit", &moved, 2),
                    *expected,
                    "{name} to {to}"
                );
            }
        }
        fs::remove_file(root.join("sub/d")).unwrap();
    }
}

fn tool_edits(target: &str) -> Vec<Value> {
    let mut edits = vec![edit("Write", target)];
    for body in [
        format!("*** Update File: {target}\n*** Move to: fresh\n+x"),
        format!("*** Update File: fresh\n*** Move to: {target}\n+x"),
    ] {
        edits.push(patch(&format!("*** Begin Patch\n{body}\n*** End Patch")));
    }
    edits
}

fn deletes(cwd: &str, name: &str, count: usize) -> Value {
    let mut payload = patch(&format!(
        "*** Begin Patch\n{}*** End Patch",
        format!("*** Delete File: {name}\n").repeat(count)
    ));
    payload["cwd"] = json!(cwd);
    payload
}

#[test]
fn edit_cwd_is_validated_before_expanding_patch_targets() {
    let dir = repo();
    let payload = deletes(&"x".repeat(4097), "a.py", 200);
    for (event, status) in EDIT_ERRORS {
        let message = String::from_utf8(hook(dir.path(), event, &payload, status)).unwrap();
        assert!(
            message.starts_with("blackhole: invalid hook cwd:"),
            "unexpected cwd diagnostic"
        );
        assert_eq!(message.lines().count(), 1);
    }
}

#[cfg(target_os = "linux")]
fn limited_hook(root: &Path, event: &str, payload: &Value, status: i32) -> String {
    let mut shell = common::command("bash");
    shell
        .args([
            "-c",
            r#"ulimit -v 262144 && exec "$1" hook "$2""#,
            "hook-test",
            env!("CARGO_BIN_EXE_blackhole"),
            event,
        ])
        .current_dir(root);
    let bytes = serde_json::to_vec(payload).unwrap();
    String::from_utf8(agent::run_input(&mut shell, &bytes, status, false).stderr).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn patch_target_batches_fit_a_bounded_address_space() {
    let dir = repo();
    for (cwd, name, count, valid) in [
        (".".to_string(), "a.py".to_string(), 1_000_000, true),
        (".".to_string(), "x".repeat(4096), 200, false),
        ("x".repeat(4097), "a.py".to_string(), 200, false),
    ] {
        let payload = deletes(&cwd, &name, count);
        for (event, error) in EDIT_ERRORS {
            let message = limited_hook(dir.path(), event, &payload, if valid { 0 } else { error });
            assert_eq!(message.lines().count(), usize::from(!valid));
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn distinct_patch_targets_under_a_long_cwd_fit_a_bounded_address_space() {
    use std::fmt::Write;
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("red.py"), "# noqa").unwrap();
    let mut cwd = root.to_owned();
    cwd.extend(std::iter::repeat_n("d".repeat(250), 14));
    fs::create_dir_all(&cwd).unwrap();
    let absent = (0..90_000).fold(String::new(), |mut text, i| {
        writeln!(text, "*** Delete File: d{i:05}/a.py").unwrap();
        text
    });
    let red = format!("*** Update File: {}\n+x\n", root.join("red.py").display());
    let expected = String::from_utf8(check(root, &["red.py"], 1)).unwrap();
    let mut payload = patch(&format!("*** Begin Patch\n{absent}{red}*** End Patch"));
    payload["cwd"] = json!(cwd);
    assert_eq!(limited_hook(root, "post-edit", &payload, 2), expected);
}

#[test]
fn hook_status_survives_closed_output_pipes() {
    use std::{io, io::Write};
    let dir = ledger_repo();
    fs::write(dir.path().join("red.py"), "# noqa").unwrap();
    for (event, data, status) in [
        (
            "pre-edit",
            edit("Write", ".blackhole/ledger").to_string(),
            2,
        ),
        ("post-edit", edit("Write", "red.py").to_string(), 2),
        ("stop", "{}".to_owned(), 2),
        ("post-edit", "{".to_owned(), 1),
    ] {
        for stderr in [true, false] {
            let (input, mut writer) = io::pipe().unwrap();
            writer.write_all(data.as_bytes()).unwrap();
            drop(writer);
            let (closed, pipe) = io::pipe().unwrap();
            drop(closed);
            let mut cmd = common::command(env!("CARGO_BIN_EXE_blackhole"));
            cmd.args(["hook", event])
                .current_dir(dir.path())
                .stdin(input);
            if stderr {
                cmd.stderr(pipe);
            } else {
                cmd.stdout(pipe);
            }
            let out = cmd.output().unwrap();
            assert_eq!(out.status.code(), Some(status), "{event} stderr={stderr}");
            if !stderr {
                assert_ne!(out.stderr, b"");
            }
            assert_eq!(out.stdout, b"");
        }
    }
}

#[test]
fn ignored_tools_skip_input_typing() {
    let dir = repo();
    for payload in [
        json!({"tool_input":{"file_path":["a.py"]},"tool_name":"Other"}),
        json!({"tool_name":"Other","cwd":[],"tool_input":7}),
    ] {
        for event in ["pre-edit", "post-edit"] {
            assert_eq!(hook(dir.path(), event, &payload, 0), b"");
        }
    }
}

#[test]
#[should_panic(expected = "incomplete test input")]
fn ordinary_run_requires_complete_input() {
    let dir = repo();
    let _ = run(
        dir.path(),
        &["hook", "stop"],
        &" ".repeat(1024 * 1024),
        0,
        false,
    );
}

#[cfg(unix)]
#[test]
fn post_edit_validates_targets_before_listing() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let dir = repo();
    let root = dir.path();
    fs::write(root.join(OsStr::from_bytes(b"non-utf8-\xff")), "").unwrap();
    let out = hook(root, "post-edit", &edit("Write", "bad\0name"), 1);
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("Cannot resolve edit target")
    );
    fs::write(root.join("a[1].py"), "# noqa").unwrap();
    let expected = run(root, &["check", "a[1].py"], "", 1, false).stdout;
    assert_eq!(
        hook(root, "post-edit", &edit("Write", "a[1].py"), 2),
        expected
    );
    let outside = edit("Write", "../outside.py");
    assert_eq!(hook(root, "post-edit", &outside, 0), b"");
    fs::write(root.join("blackhole.toml"), "broken = [").unwrap();
    hook(root, "post-edit", &outside, 1);
}

#[test]
fn post_edit_judges_file_names_with_packages_like_check() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir_all(root.join("tests/pkg")).unwrap();
    for name in ["tests/pkg/package.json", "tests/pkg/good.ts", "a.specs.ts"] {
        fs::write(root.join(name), "{}").unwrap();
    }
    let red = check(root, &["a.specs.ts"], 1);
    assert!(String::from_utf8_lossy(&red).contains("a.specs.ts:1:1 ts/test-file-names"));
    assert_eq!(
        hook(root, "post-edit", &edit("Write", "a.specs.ts"), 2),
        red
    );
    assert_eq!(check(root, &["tests/pkg/good.ts"], 0), b"");
    assert_eq!(
        hook(root, "post-edit", &edit("Write", "tests/pkg/good.ts"), 0),
        b""
    );
}
