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
    fs::write(root.join("a.py"), "pass").unwrap();
    // A broken policy file is the author's to fix, so the loop guard does not release it.
    for (event, payload) in [
        ("stop", json!({})),
        ("stop", json!({"stop_hook_active":true})),
        ("post-edit", edit("Write", "a.py")),
    ] {
        let error = String::from_utf8(hook(root, event, &payload, 2)).unwrap();
        assert!(error.starts_with("blackhole: blackhole.toml:"), "{event}");
        assert_eq!(error.lines().count(), 1);
    }
}

#[test]
fn blocking_hooks_block_when_git_reports_the_wrong_checkout_root() {
    let dir = repo();
    let root = dir.path();
    // Exit 1 needs a directory with no `.git` above it, which the default temporary directory may not be.
    let outside = tempfile::tempdir_in("/tmp").unwrap();
    common::git(
        root,
        &["config", "core.worktree", outside.path().to_str().unwrap()],
    )
    .unwrap();
    // The checkout's own git configuration is the author's to fix, so the loop guard does not release it.
    for (event, payload) in [
        ("stop", json!({})),
        ("stop", json!({"stop_hook_active":true})),
        ("pre-edit", edit("Write", "a.py")),
        ("post-edit", edit("Write", "a.py")),
    ] {
        let error = String::from_utf8(hook(root, event, &payload, 2)).unwrap();
        assert!(
            error.starts_with("blackhole: checkout root mismatch"),
            "{event}: {error}"
        );
        assert_eq!(error.lines().count(), 1);
        assert_ne!(hook(outside.path(), event, &payload, 1), b"");
    }
}

#[test]
fn stop_reads_its_whole_payload_before_blocking() {
    let dir = repo();
    let outside = tempdir().unwrap();
    common::git(
        dir.path(),
        &["config", "core.worktree", outside.path().to_str().unwrap()],
    )
    .unwrap();
    // A payload larger than a pipe buffer fails to arrive unless the hook drains stdin.
    let payload = json!({"stop_hook_active": false, "pad": "x".repeat(1 << 20)});
    assert_ne!(hook(dir.path(), "stop", &payload, 2), b"");
}

fn stop_without_git(root: &Path, active: bool, status: i32) -> String {
    let empty = tempdir().unwrap();
    let mut cmd = common::command(env!("CARGO_BIN_EXE_blackhole"));
    cmd.args(["hook", "stop"])
        .current_dir(root)
        .env("PATH", empty.path());
    let payload = json!({"stop_hook_active":active}).to_string();
    let out = agent::run_input(&mut cmd, payload.as_bytes(), status, false);
    assert_eq!(out.stdout, b"");
    String::from_utf8(out.stderr).unwrap()
}

#[cfg(unix)]
#[test]
fn the_loop_guard_releases_only_when_the_environment_fails() {
    use std::os::unix::fs::PermissionsExt;
    let active = json!({"stop_hook_active":true});
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink("loop.py", root.join("loop.py")).unwrap();
    let error = String::from_utf8(hook(root, "stop", &active, 2)).unwrap();
    assert!(
        error.starts_with("blackhole: cannot read loop.py: ") && error.contains("remove it"),
        "{error}"
    );
    fs::remove_file(root.join("loop.py")).unwrap();

    let secret = root.join("secret.py");
    fs::write(&secret, "pass").unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    // A privileged user reads the file anyway, and then there is nothing to refuse.
    if fs::File::open(&secret).is_err() {
        let error = String::from_utf8(hook(root, "stop", &active, 2)).unwrap();
        assert!(
            error.starts_with("blackhole: cannot read secret.py: "),
            "{error}"
        );
    }
    fs::remove_file(&secret).unwrap();

    fs::write(root.join(".git/config"), "[broken").unwrap();
    common::git(root, &["status"]).unwrap_err();
    let error = String::from_utf8(hook(root, "stop", &active, 2)).unwrap();
    assert!(
        error.starts_with("blackhole: ") && !error.contains("no verdict"),
        "{error}"
    );
    fs::write(root.join(".git/config"), "").unwrap();

    // Claude and Codex set this flag when a stop hook already continued the turn, so an outage cannot loop forever.
    let released = stop_without_git(root, true, 0);
    assert!(
        released.starts_with("blackhole: no verdict: ")
            && released
                .trim_end()
                .ends_with("the judgment could not run, so the agent may stop"),
        "{released}"
    );
    assert_eq!(released.lines().count(), 1);
    let blocked = stop_without_git(root, false, 2);
    assert!(blocked.starts_with("blackhole: no verdict: "), "{blocked}");
}

#[cfg(unix)]
#[test]
fn check_resolution_errors_escape_the_path() {
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink("loop\n", root.join("loop\n")).unwrap();
    // A looping link is the checkout's to fix, so it is red and names the path.
    let out = run(root, &["check", "loop\n/file.py"], "", 1, false);
    assert_eq!(out.stderr, b"");
    let error = String::from_utf8(out.stdout).unwrap();
    assert!(
        error.contains(r"loop\n/file.py") && error.contains("remove it"),
        "{error:?}"
    );
    assert_eq!(error.lines().count(), 1);
}

#[cfg(unix)]
#[test]
fn edit_targets_keep_the_class_of_their_filesystem_failure() {
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink("loop.py", root.join("loop.py")).unwrap();
    for event in ["pre-edit", "post-edit"] {
        let error = String::from_utf8(hook(root, event, &edit("Write", "loop.py"), 2)).unwrap();
        assert!(
            error.starts_with("blackhole: cannot resolve ")
                && error.contains("loop.py")
                && error.contains("remove it"),
            "{event}: {error}"
        );
    }
}

#[test]
fn a_malformed_git_marker_is_not_outside_a_checkout() {
    // The default temporary directory can sit under a stray `.git`, and this test needs one it controls.
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    assert_ne!(hook(dir.path(), "stop", &json!({}), 2), b"");
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
    let outside = tempfile::tempdir_in("/tmp").unwrap();
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
            message.starts_with("blackhole: cannot resolve hook cwd "),
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
    // `check` never reads stdin, so the harness must report the input it could not deliver.
    let _ = run(dir.path(), &["check"], &" ".repeat(1024 * 1024), 0, false);
}

#[cfg(unix)]
#[test]
fn post_edit_validates_targets_before_listing() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let dir = repo();
    let root = dir.path();
    fs::write(root.join(OsStr::from_bytes(b"non-utf8-\xff")), "").unwrap();
    let out = hook(root, "post-edit", &edit("Write", "bad\0name"), 1);
    assert!(String::from_utf8(out).unwrap().contains("cannot resolve "));
    fs::write(root.join("a[1].py"), "# noqa").unwrap();
    let expected = run(root, &["check", "a[1].py"], "", 1, false).stdout;
    assert_eq!(
        hook(root, "post-edit", &edit("Write", "a[1].py"), 2),
        expected
    );
    let outside = edit("Write", "../outside.py");
    assert_eq!(hook(root, "post-edit", &outside, 0), b"");
    fs::write(root.join("blackhole.toml"), "broken = [").unwrap();
    let broken = hook(root, "post-edit", &outside, 2);
    assert!(broken.starts_with(b"blackhole: blackhole.toml:"));
}

#[cfg(unix)]
fn git_shim(dir: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let real = String::from_utf8(
        common::command("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let shim = dir.join("git");
    fs::write(
        &shim,
        format!(
            "#!/bin/sh\nreal='{}'\n{script}\nexec \"$real\" \"$@\"\n",
            real.trim_end()
        ),
    )
    .unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
}

// A git that the machine stops with a signal never answered, whatever the checkout holds.
#[cfg(unix)]
fn git_killed_on(dir: &Path, subcommand: &str, signal: &str, once: bool) {
    let state = dir.join("killed");
    let guard = if once {
        format!(
            "[ -e '{0}' ] || {{ : > '{0}'; kill -{signal} $$; }}",
            state.display()
        )
    } else {
        format!("kill -{signal} $$")
    };
    let request = if subcommand == "cat-file" {
        "IFS= read -r request; "
    } else {
        ""
    };
    git_shim(
        dir,
        &format!(r#"case " $* " in *" {subcommand} "*) {request}{guard};; esac"#),
    );
}

#[cfg(unix)]
#[test]
fn check_keeps_no_verdict_for_a_stopped_machine() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.ts"), "export const a = 1;\n").unwrap();
    let shims = tempdir().unwrap();
    // Only a signal from outside is an outage; any other could come from what git read.
    for (signal, code) in [("KILL", 75), ("TERM", 75), ("USR1", 1)] {
        git_killed_on(shims.path(), "ls-files", signal, false);
        let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .env("PATH", shims.path())
            .arg("check")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(code), "{signal}: {out:?}");
    }
    // A parser thread the system cannot start is not a finding in the file.
    let out = common::command("sh")
        .current_dir(root)
        .args(["-c", "ulimit -v 700000 && exec \"$0\" check"])
        .arg(env!("CARGO_BIN_EXE_blackhole"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(75), "{out:?}");
    assert_eq!(out.stdout, b"", "{out:?}");
}

#[cfg(unix)]
#[test]
fn ci_keeps_the_signal_when_git_stops_during_a_head_blob_read() {
    let dir = repo();
    let root = dir.path();
    common::git(root, &["commit", "--allow-empty", "-qm", "base"]).unwrap();
    fs::write(root.join("a.py"), "pass\n").unwrap();
    common::git(root, &["add", "a.py"]).unwrap();
    common::git(root, &["commit", "-qm", "head"]).unwrap();
    let shims = tempdir().unwrap();
    for (signal, status, expected) in [
        ("USR1", 1, "git crashed: "),
        ("KILL", 75, "blackhole ci: no verdict: git did not finish: "),
    ] {
        git_killed_on(shims.path(), "cat-file", signal, false);
        let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .env("PATH", shims.path())
            .args(["ci", "--base", "HEAD~", "--head", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(status), "{signal}: {out:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert!(stdout.starts_with(expected), "{signal}: {stdout}");
        if status == 1 {
            assert_eq!(
                stdout.lines().last(),
                Some("blackhole ci: red (unreadable)")
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn coverage_keeps_a_head_blob_crash_red_and_preserves_pending_gates() {
    let dir = repo();
    let root = dir.path();
    let config = "[coverage]\nlcov = \"reports/lcov.info\"\n";
    fs::write(root.join("blackhole.toml"), config).unwrap();
    common::git(root, &["add", "-A"]).unwrap();
    common::git(root, &["commit", "-qm", "base"]).unwrap();
    fs::write(root.join("blackhole.toml"), format!("# head\n{config}")).unwrap();
    fs::write(root.join("a.py"), "pass\n").unwrap();
    common::git(root, &["add", "-A"]).unwrap();
    common::git(root, &["commit", "-qm", "head"]).unwrap();
    let blob = common::git(root, &["rev-parse", "HEAD:a.py"]).unwrap();
    let shims = tempdir().unwrap();
    git_shim(
        shims.path(),
        &format!(
            r#"case " $* " in *" cat-file "*)
                while IFS= read -r request; do
                    [ "${{request##* }}" != '{blob}' ] || kill -USR1 $$
                    printf '%s\n' "$request" | "$real" "$@" || exit $?
                done
                exit 0;;
            esac"#
        ),
    );
    let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(root)
        .env("PATH", shims.path())
        .args(["ci", "--base", "HEAD~", "--head", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("git crashed: "), "{stdout}");
    assert_eq!(
        stdout.lines().last(),
        Some("blackhole ci: red (unreadable); waiting (protected)")
    );
}

#[cfg(unix)]
#[test]
fn a_retry_judges_from_the_starting_directory() {
    let dir = repo();
    let root = dir.path();
    let _ = run(root, &["init"], "", 0, false);
    fs::write(root.join("a.py"), "# noqa").unwrap();
    let expected = check(root, &["a.py"], 1);
    let shims = tempdir().unwrap();
    // The first listing runs after the policy is read from inside `.blackhole`.
    git_killed_on(shims.path(), "ls-files", "KILL", true);
    let mut cmd = common::command(env!("CARGO_BIN_EXE_blackhole"));
    cmd.args(["hook", "post-edit"])
        .current_dir(root)
        .env("PATH", shims.path());
    let payload = edit("Write", "a.py").to_string();
    let out = agent::run_input(&mut cmd, payload.as_bytes(), 2, false);
    assert!(shims.path().join("killed").exists());
    assert_eq!(out.stderr, expected);
}

#[cfg(unix)]
#[test]
fn a_looping_hook_cwd_is_red() {
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink("loop", root.join("loop")).unwrap();
    let mut payload = edit("Write", "a.py");
    payload["cwd"] = json!("loop");
    for event in ["pre-edit", "post-edit"] {
        let error = String::from_utf8(hook(root, event, &payload, 2)).unwrap();
        assert!(
            error.starts_with("blackhole: cannot resolve hook cwd loop: ")
                && error.contains("remove it"),
            "{event}: {error}"
        );
    }
}

#[cfg(unix)]
#[test]
fn coverage_keeps_an_outage_apart_from_missing_evidence() {
    let dir = repo();
    let root = dir.path();
    fs::write(
        root.join("blackhole.toml"),
        "[coverage]\nlcov = \"reports/lcov.info\"\n",
    )
    .unwrap();
    common::git(root, &["add", "-A"]).unwrap();
    common::git(root, &["commit", "-qm", "base"]).unwrap();
    fs::write(root.join("a.py"), "x = 1\n").unwrap();
    common::git(root, &["add", "-A"]).unwrap();
    common::git(root, &["commit", "-qm", "head"]).unwrap();
    let head = common::git(root, &["rev-parse", "HEAD"]).unwrap();
    let reports = tempdir().unwrap();
    let report = reports.path().join(&head).join("reports");
    fs::create_dir_all(&report).unwrap();
    fs::write(report.join("lcov.info"), "SF:a.py\nDA:1,1\nend_of_record\n").unwrap();
    let shims = tempdir().unwrap();
    // The checkout lookup runs only for the report, so a stopped git there is an outage, not missing coverage.
    git_killed_on(shims.path(), "--is-bare-repository", "KILL", false);
    let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(root)
        .env("PATH", shims.path())
        .args(["ci", "--base", "HEAD~", "--head", "HEAD", "--coverage-dir"])
        .arg(reports.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(75), "{out:?}");
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .starts_with("blackhole ci: no verdict: ")
    );
}

#[cfg(unix)]
#[test]
fn check_and_stop_report_a_tracked_fifo() {
    let dir = repo();
    let root = dir.path();
    let path = root.join("a.py");
    fs::write(&path, "pass\n").unwrap();
    common::git(root, &["add", "a.py"]).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        common::command("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let [checked, stopped] = [&["check"][..], &["hook", "stop"]].map(|args| {
        common::command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .args(args)
            .output()
            .unwrap()
    });
    assert_eq!(
        [checked.status.code(), stopped.status.code()],
        [Some(1), Some(2)],
        "{checked:?} {stopped:?}"
    );
    assert!(
        checked.stdout.starts_with(b"blackhole: cannot read a.py: "),
        "{checked:?}"
    );
    assert_eq!(stopped.stderr, checked.stdout);
    assert_eq!(
        hook(root, "stop", &json!({"stop_hook_active":true}), 2),
        checked.stdout
    );
}

#[cfg(unix)]
#[test]
fn ci_finishes_with_a_verdict_when_git_stops_resolving_a_revision() {
    let dir = repo();
    let root = dir.path();
    common::git(root, &["commit", "--allow-empty", "-qm", "base"]).unwrap();
    let shims = tempdir().unwrap();
    for (signal, status) in [
        ("USR1", 1),
        ("HUP", 75),
        ("INT", 75),
        ("KILL", 75),
        ("TERM", 75),
    ] {
        git_killed_on(shims.path(), "rev-parse", signal, false);
        let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .env("PATH", shims.path())
            .args(["ci", "--base", "HEAD", "--head", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(status), "{signal}: {out:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        let last = stdout.lines().last().unwrap();
        if status == 1 {
            assert!(stdout.starts_with("git crashed: "), "{stdout}");
            assert_eq!(last, "blackhole ci: red (unreadable)", "{stdout}");
        } else {
            assert!(
                last.starts_with("blackhole ci: no verdict: git did not finish: "),
                "{signal}: {stdout}"
            );
        }
    }
}

#[cfg(unix)]
fn with_deadline(root: &Path, args: &[&str], block_input: bool) -> std::process::Output {
    common::command("python3")
        .args([
            "-I",
            "-c",
            r"
import os
import signal
import subprocess
import sys

stdin = subprocess.PIPE if sys.argv[1] == 'block' else subprocess.DEVNULL
with subprocess.Popen(sys.argv[2:], stdin=stdin, stdout=subprocess.PIPE,
                      stderr=subprocess.PIPE, start_new_session=True) as proc:
    try:
        proc.wait(timeout=10)
        stdout, stderr = proc.communicate(timeout=2)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        sys.exit('blackhole missed its deadline or left a child running')
sys.stdout.buffer.write(stdout)
sys.stderr.buffer.write(stderr)
sys.exit(proc.returncode)
",
            if block_input { "block" } else { "eof" },
            env!("CARGO_BIN_EXE_blackhole"),
        ])
        .args(args)
        .env("BLACKHOLE_TEST_DEADLINE_MS", "1000")
        .current_dir(root)
        .output()
        .unwrap()
}

#[cfg(unix)]
#[test]
fn check_and_stop_hit_the_deadline_when_git_cannot_finish_listing() {
    let dir = repo();
    let root = dir.path();
    assert!(
        common::command("mkfifo")
            .arg(root.join(".gitignore"))
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.join("a.py"), "pass\n").unwrap();
    let checked = with_deadline(root, &["check"], false);
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    assert_eq!(
        checked.stderr,
        b"blackhole: deadline exceeded while running git ls-files\n"
    );

    let stopped = with_deadline(root, &["hook", "stop"], false);
    assert_eq!(stopped.status.code(), Some(2), "{stopped:?}");
    assert_eq!(
        stopped.stderr,
        b"blackhole: deadline exceeded while running git ls-files\n"
    );
}

#[cfg(unix)]
#[test]
fn a_hook_deadline_also_covers_a_blocked_read() {
    let dir = repo();
    let out = with_deadline(dir.path(), &["hook", "stop"], true);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(
        out.stderr,
        b"blackhole: deadline exceeded while reading the hook payload\n"
    );
}

#[cfg(unix)]
#[test]
fn coverage_checks_both_blob_sizes_before_requesting_their_bodies() {
    let dir = repo();
    let root = dir.path();
    fs::write(
        root.join("blackhole.toml"),
        "[coverage]\nlcov = \"reports/lcov.info\"\n",
    )
    .unwrap();
    fs::write(root.join("a.py"), "x = 1\n").unwrap();
    common::git(root, &["add", "-A"]).unwrap();
    common::git(root, &["commit", "-qm", "base"]).unwrap();
    fs::write(root.join("a.py"), "x = 2\n").unwrap();
    common::git(root, &["add", "-A"]).unwrap();
    common::git(root, &["commit", "-qm", "head"]).unwrap();
    let shims = tempdir().unwrap();
    git_shim(
        shims.path(),
        r#"case " $* " in *" cat-file "*)
            while IFS= read -r request; do
                case "$request" in
                    "info $BLOB") printf '%s blob %s\n' "$BLOB" "$SIZE"; continue;;
                    "$BLOB"|"contents $BLOB")
                        if [ "$SIZE" -gt 1072693248 ]; then
                            : > "$BODY"
                            exit 42
                        fi;;
                esac
                printf '%s\n' "$request" | "$real" "$@" || exit $?
            done
            exit 0;;
        esac"#,
    );
    for rev in ["HEAD~:a.py", "HEAD:a.py"] {
        let blob = common::git(root, &["rev-parse", rev]).unwrap();
        for (size, expected) in [
            ("1072693248", "trusted report missing"),
            ("1072693249", "a.py exceeds Git's text diff size limit"),
        ] {
            let body = shims.path().join("body-requested");
            let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
                .current_dir(root)
                .env("PATH", shims.path())
                .env("BLOB", &blob)
                .env("SIZE", size)
                .env("BODY", &body)
                .args(["ci", "--base", "HEAD~", "--head", "HEAD"])
                .output()
                .unwrap();
            assert!(!body.exists(), "{rev} at {size}: {out:?}");
            assert_eq!(out.status.code(), Some(1), "{rev} at {size}: {out:?}");
            let stdout = String::from_utf8(out.stdout).unwrap();
            assert!(stdout.contains(expected), "{rev} at {size}: {stdout}");
            assert_eq!(stdout.lines().last(), Some("blackhole ci: red (coverage)"));
        }
    }
}

#[test]
fn check_and_stop_distinguish_a_replaced_tracked_file_from_a_gitlink() {
    let dir = repo();
    let root = dir.path();
    common::git(root, &["commit", "--allow-empty", "-qm", "base"]).unwrap();
    let base = common::git(root, &["rev-parse", "HEAD"]).unwrap();
    common::git(
        root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{base},module"),
        ],
    )
    .unwrap();
    fs::create_dir(root.join("module")).unwrap();
    assert_eq!(check(root, &[], 0), b"");
    assert_eq!(hook(root, "stop", &json!({}), 0), b"");

    let path = root.join("a.py");
    fs::write(&path, "pass\n").unwrap();
    common::git(root, &["add", "a.py"]).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let [checked, stopped] = [&["check"][..], &["hook", "stop"]].map(|args| {
        common::command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .args(args)
            .output()
            .unwrap()
    });
    assert_eq!(
        [checked.status.code(), stopped.status.code()],
        [Some(1), Some(2)],
        "{checked:?} {stopped:?}"
    );
    assert!(
        checked.stdout.starts_with(b"blackhole: cannot read a.py: "),
        "{checked:?}"
    );
    assert_eq!(stopped.stderr, checked.stdout);
}
