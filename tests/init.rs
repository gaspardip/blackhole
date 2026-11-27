#[path = "common/agent.rs"]
mod agent;
#[path = "common/cli.rs"]
mod cli;
pub mod common;
#[path = "common/gh.rs"]
mod gh;
use agent::run;
use cli::{check, repo};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::tempdir;

const HOOK_FILES: [&str; 2] = [".claude/settings.json", ".codex/hooks.json"];
const WORKFLOW: &str = ".github/workflows/blackhole.yml";

const ISOLATED_ROOT: &str = r#"names=$(command -p env | LC_ALL=C command -p sed -n 's/^\(GIT_[A-Za-z0-9_]*\)=.*/\1/p') || exit
while IFS= read -r name; do
    [ -z "$name" ] || unset "$name" || exit
done <<BLACKHOLE_GIT_ENV
$names
BLACKHOLE_GIT_ENV
exec git --no-replace-objects --no-lazy-fetch rev-parse --show-toplevel"#;

fn old_launcher(root: &str, event: &str) -> String {
    format!(
        r#"if root=$({root} 2>/dev/null) && local_bin="$root/node_modules/.bin/blackhole" && [ -f "$local_bin" ] && [ -x "$local_bin" ]; then exec "$local_bin" hook {event}; fi; exec blackhole hook {event}"#
    )
}

fn previous_launcher(event: &str) -> String {
    old_launcher("git --no-lazy-fetch rev-parse --show-toplevel", event)
}

const NO_REMOTE: &str = "not guarded: origin names no GitHub repository (add the GitHub repository as the origin remote)";

// A checkout without a GitHub remote completes the install and reads as not guarded without calling gh.
fn init(root: &Path, success: bool) -> String {
    let fake = gh::FakeGh::new();
    let out = agent::run_input(
        common::command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .arg("init")
            .env("PATH", fake.path()),
        b"",
        if success { 1 } else { 2 },
        false,
    );
    assert_eq!(fake.calls(), Vec::<String>::new());
    // A run that fails reports no file as written.
    assert!(success || out.stdout.is_empty(), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !success || stdout.ends_with(&format!("\n{NO_REMOTE}\n")),
        "{stdout}"
    );
    if success {
        stdout
    } else {
        String::from_utf8(out.stderr).unwrap()
    }
}
#[test]
fn installs_and_merges_once() {
    let dir = repo();
    let root = dir.path();
    let original = json!({
        "description":"Existing settings",
        "hooks":{
            "Stop":[
                {"hooks":[{"type":"command","command":"first"}]},
                {"hooks":[
                    {"type":"command","command":"blackhole hook stop"},
                    {"type":"command","command":previous_launcher("stop")},
                    {"type":"command","command":old_launcher(ISOLATED_ROOT, "stop")},
                    {"type":"command","command":"second"}
                ]}
            ],
            "SessionStart":[{"hooks":[{"type":"command","command":"session"}]}]
        }
    });
    fs::write(root.join("blackhole.toml"), "exclude=[]").unwrap();
    for name in HOOK_FILES {
        let p = root.join(name);
        fs::create_dir(p.parent().unwrap()).unwrap();
        fs::write(p, original.to_string()).unwrap();
    }
    assert!(init(root, true).contains("The judge is the blackhole check"));
    assert_eq!(
        fs::read(root.join("blackhole.toml")).unwrap(),
        b"exclude=[]"
    );
    assert_eq!(
        fs::read(root.join(".blackhole/ledger")).unwrap(),
        b"blackhole-ledger 1\n"
    );
    for (name, matcher) in HOOK_FILES
        .into_iter()
        .zip(["^(Edit|Write|MultiEdit|NotebookEdit)$", "^apply_patch$"])
    {
        let p = root.join(name);
        let bytes = fs::read(&p).unwrap();
        let doc: Value = serde_json::from_slice(&bytes).unwrap();
        let stop = &doc["hooks"]["Stop"][1]["hooks"][0]["command"];
        assert_ne!(stop, "blackhole hook stop");
        assert_ne!(stop, &Value::String(previous_launcher("stop")));
        assert_ne!(stop, &Value::String(old_launcher(ISOLATED_ROOT, "stop")));
        let mut upgraded = original.clone();
        for index in [0, 1, 2] {
            upgraded["hooks"]["Stop"][1]["hooks"][index]["command"] = stop.clone();
        }
        for event in ["PreToolUse", "PostToolUse"] {
            let groups = doc["hooks"][event].as_array().unwrap();
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0]["matcher"], matcher);
            assert_eq!(groups[0]["hooks"][0]["type"], "command");
            upgraded["hooks"][event] = doc["hooks"][event].clone();
        }
        assert_eq!(doc, upgraded);
        init(root, true);
        assert_eq!(fs::read(p).unwrap(), bytes);
    }
}

// A PATH directory that holds git and nothing else.
fn git_only_path() -> tempfile::TempDir {
    let ambient = tempdir().unwrap();
    let git = common::command("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(git.status.success());
    std::os::unix::fs::symlink(
        String::from_utf8(git.stdout).unwrap().trim_end(),
        ambient.path().join("git"),
    )
    .unwrap();
    ambient
}

#[test]
fn generated_hooks_use_the_checkout_install_or_the_global_binary() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let root = &dir.path().join("checkout with spaces");
    let subdir = root.join("nested directory");
    fs::create_dir_all(&subdir).unwrap();
    common::git(root, &["init", "-q"]).unwrap();
    fs::write(root.join(".gitignore"), "/node_modules/\n").unwrap();
    init(root, true);
    fs::write(root.join("red.py"), "# noqa").unwrap();
    let red = check(root, &["red.py"], 1).stdout;
    assert!(red.starts_with(b"red.py:1:1 "));

    let other = repo();
    let other_git = other.path().join(".git");
    let other_bin = other.path().join("node_modules/.bin/blackhole");
    fs::create_dir_all(other_bin.parent().unwrap()).unwrap();
    symlink("/bin/true", other_bin).unwrap();
    let git_env = [
        ("GIT_DIR", other_git.as_path()),
        ("GIT_WORK_TREE", other.path()),
        ("GIT_CEILING_DIRECTORIES", root.as_path()),
    ];
    let direct = agent::run_input(
        common::command(env!("CARGO_BIN_EXE_blackhole"))
            .args(["hook", "stop"])
            .current_dir(&subdir)
            .envs(git_env),
        b"{}",
        2,
        false,
    );
    assert!(direct.stderr.starts_with(&red));

    let ambient = git_only_path();

    let local = root.join("node_modules/.bin/blackhole");
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    for bin in [&local, &ambient.path().join("blackhole")] {
        symlink(env!("CARGO_BIN_EXE_blackhole"), bin).unwrap();
        for (name, tool) in HOOK_FILES.into_iter().zip(["Write", "apply_patch"]) {
            let doc: Value = serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap();
            let payload = |path: &str| {
                if tool == "Write" {
                    json!({"tool_name":tool,"cwd":root,"tool_input":{"file_path":path}})
                } else {
                    json!({"tool_name":tool,"cwd":root,"tool_input":{"command":format!(
                        "*** Begin Patch\n*** Update File: {path}\n+x\n*** End Patch"
                    )}})
                }
            };
            for (event, input, expected) in [
                ("Stop", json!({}), red.as_slice()),
                (
                    "PreToolUse",
                    payload(".blackhole/ledger"),
                    b"blackhole: The ledger".as_slice(),
                ),
                ("PostToolUse", payload("red.py"), red.as_slice()),
            ] {
                let command = doc["hooks"][event][0]["hooks"][0]["command"]
                    .as_str()
                    .unwrap();
                assert!(!command.contains(root.to_str().unwrap()));
                let output = agent::run_input(
                    common::command("/bin/sh")
                        .args(["-c", command])
                        .current_dir(&subdir)
                        .env("PATH", ambient.path())
                        .envs(git_env),
                    input.to_string().as_bytes(),
                    2,
                    false,
                );
                assert!(
                    output.stderr.starts_with(expected),
                    "{name} {event}: {output:?}"
                );
            }
        }
        fs::remove_file(bin).unwrap();
    }
}
#[test]
fn invalid_settings_are_preserved() {
    for (file, other) in [
        (HOOK_FILES[0], HOOK_FILES[1]),
        (HOOK_FILES[1], HOOK_FILES[0]),
    ] {
        let dir = repo();
        let root = dir.path();
        fs::create_dir(root.join(file).parent().unwrap()).unwrap();
        let invalid = [
            r#"{"$serde_json::private::Number":"1.25"}"#,
            r#"{"other":{"\u0024serde_json::private::Number":"1.25"}}"#,
            r#"{"other":[{"$serde_json::private::RawValue":"hi"}]}"#,
            r#"{"hooks":[]}"#,
            r#"{"hooks":{"Stop":{}}}"#,
            r#"{"extra":[{"x":1,"\u0078":2}]}"#,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command"}]}]}}"#,
        ];
        let configuration = invalid.map(|text| (text, "unexpected hooks configuration"));
        for (bad, reason) in [("{", "EOF")].into_iter().chain(configuration) {
            let p = root.join(file);
            fs::write(&p, bad).unwrap();
            let message = init(root, false);
            assert!(message.contains(file));
            assert!(message.contains(reason), "{message}");
            assert_eq!(fs::read_to_string(p).unwrap(), bad);
            assert!(!root.join(other).exists());
        }
    }
}
#[test]
fn malformed_ledger_stops_init_before_any_write() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".blackhole")).unwrap();
    fs::write(root.join(".blackhole/ledger"), "not a ledger\n").unwrap();
    let message = init(root, false);
    assert!(message.contains("ledger line 1"), "{message}");
    assert_eq!(message.lines().count(), 1);
    for name in HOOK_FILES.into_iter().chain(["blackhole.toml"]) {
        assert!(!root.join(name).exists(), "{name}");
    }
    assert_eq!(
        fs::read(root.join(".blackhole/ledger")).unwrap(),
        b"not a ledger\n"
    );
}

#[cfg(unix)]
#[test]
fn outside_settings_directory() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir(&root).unwrap();
    common::git(&root, &["init", "-q"]).unwrap();
    std::os::unix::fs::symlink(dir.path(), root.join(".claude")).unwrap();
    assert!(init(&root, false).contains(".claude"));
    assert!(!dir.path().join("settings.json").exists());
}

#[test]
fn initializes_empty_repository_and_keeps_first_ledger() {
    let dir = repo();
    let root = dir.path();
    let report = init(root, true);
    for path in [
        "blackhole.toml",
        ".blackhole/ledger",
        ".claude/settings.json",
        ".codex/hooks.json",
    ] {
        let matches = report.lines().filter(|line| line.ends_with(path));
        assert_eq!(matches.count(), 1);
    }
    assert_eq!(fs::read(root.join("blackhole.toml")).unwrap(), b"");
    fs::write(root.join("new.py"), "# noqa").unwrap();
    init(root, true);
    assert_eq!(
        fs::read(root.join(".blackhole/ledger")).unwrap(),
        b"blackhole-ledger 1\n"
    );
}

#[test]
fn malformed_objections_stop_init_before_any_write() {
    let dir = repo();
    let root = dir.path();
    init(root, true);
    for name in HOOK_FILES.into_iter().chain(["blackhole.toml"]) {
        fs::remove_file(root.join(name)).unwrap();
    }
    fs::write(root.join(".blackhole/objections"), "invalid").unwrap();
    let message = init(root, false);
    assert!(message.contains("objections line 1"), "{message}");
    for name in HOOK_FILES.into_iter().chain(["blackhole.toml"]) {
        assert!(!root.join(name).exists(), "{name}");
    }
    assert_eq!(
        fs::read(root.join(".blackhole/ledger")).unwrap(),
        b"blackhole-ledger 1\n"
    );
}

#[cfg(unix)]
#[test]
fn refuses_internal_settings_directory_links() {
    for name in [".claude", ".codex"] {
        let dir = repo();
        let root = dir.path();
        fs::create_dir(root.join("settings")).unwrap();
        std::os::unix::fs::symlink("settings", root.join(name)).unwrap();
        assert!(init(root, false).contains(name));
        assert_eq!(fs::read_dir(root.join("settings")).unwrap().count(), 0);
    }
}

#[test]
fn initializes_from_subdirectory() {
    let dir = repo();
    let root = dir.path();
    let sub = root.join("sub");
    fs::create_dir(&sub).unwrap();
    init(&sub, true);
    assert!(root.join(".blackhole/ledger").is_file());
    assert_eq!(fs::read_dir(sub).unwrap().count(), 0);
}

#[test]
fn separate_git_directory_is_untouched() {
    fn entries(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                paths.extend(entries(&path));
            }
            paths.push(path);
        }
        paths.sort();
        paths
    }
    let temp = tempdir().unwrap();
    let external = temp.path().join("git");
    common::git(temp.path(), &["init", "--separate-git-dir", "git", "tree"]).unwrap();
    let before = entries(&external);
    init(&temp.path().join("tree"), true);
    assert_eq!(entries(&external), before);
}

#[test]
fn initial_ledger_banks_only_allowable_findings() {
    let temp = repo();
    let root = temp.path();
    fs::write(root.join("ordinary.py"), "# noqa").unwrap();
    fs::write(root.join("broken.ts"), "const = ;").unwrap();
    let message = init(root, true);
    assert!(
        message
            .lines()
            .any(|l| l.contains("1 finding") && l.contains("blackhole check"))
    );
    assert_eq!(
        fs::read_to_string(root.join(".blackhole/ledger")).unwrap(),
        "blackhole-ledger 1\ncore/no-suppression\t1\tordinary.py\n"
    );
    let out = check(root, &[], 1);
    let findings = String::from_utf8(out.stdout).unwrap();
    assert!(findings.contains("core/unparseable"));
    assert!(!findings.contains("ordinary.py"));
    let stop = run(root, &["hook", "stop"], "{}", 2, false);
    assert!(stop.stderr.starts_with(findings.as_bytes()));
}

#[test]
fn settings_keep_number_precision_and_key_order() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".claude")).unwrap();
    let file = root.join(".claude/settings.json");
    fs::write(
        &file,
        r#"{"z":{"y":0.12345678901234567890123456789,"b":true},"a":1}"#,
    )
    .unwrap();
    init(root, true);
    let text = fs::read_to_string(file).unwrap();
    assert!(text.starts_with("{\n  \"z\": {\n    \"y\": 0.12345678901234567890123456789,\n    \"b\": true\n  },\n  \"a\": 1,\n  \"hooks\":"));
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["hooks"].as_object().unwrap().len(), 3);
}

#[test]
fn settings_keep_original_number_tokens() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".claude")).unwrap();
    let file = root.join(".claude/settings.json");
    let prefix = r#"{
  "values": [
    1E2,
    1e2,
    -0,
    -0.0,
    "ignore \"2E3\"",
    1.50,
    12345678901234567890123
  ]"#;
    fs::write(&file, format!("{prefix}\n}}\n")).unwrap();
    init(root, true);
    let text = fs::read_to_string(file).unwrap();
    assert!(
        text.starts_with(&format!("{prefix},\n  \"hooks\":")),
        "{text}"
    );
}

#[test]
fn settings_keep_numbers_outside_the_float_range() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".claude")).unwrap();
    let file = root.join(".claude/settings.json");
    let prefix = "{\n  \"values\": [\n    1e400,\n    -1E+400,\n    1e-400\n  ]";
    fs::write(&file, format!("{prefix}\n}}\n")).unwrap();
    init(root, true);
    let text = fs::read_to_string(file).unwrap();
    assert!(
        text.starts_with(&format!("{prefix},\n  \"hooks\":")),
        "{text}"
    );
}

#[test]
fn codex_accepts_null_matcher() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".codex")).unwrap();
    let file = root.join(HOOK_FILES[1]);
    let original = json!({"matcher":null,"hooks":[{"type":"command","command":"existing"}]});
    fs::write(&file, json!({"hooks":{"Stop":[original]}}).to_string()).unwrap();
    init(root, true);
    let installed: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    assert_eq!(installed["hooks"]["Stop"][0], original);
    assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 2);
}

#[test]
fn codex_accepts_omitted_group_hooks() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".codex")).unwrap();
    let file = root.join(HOOK_FILES[1]);
    let original = json!({});
    fs::write(&file, json!({"hooks":{"Stop":[original]}}).to_string()).unwrap();
    init(root, true);
    let installed: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    assert_eq!(installed["hooks"]["Stop"][0], original);
    assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 2);
}

#[test]
fn codex_rejects_unknown_top_level_keys() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".codex")).unwrap();
    let file = root.join(HOOK_FILES[1]);
    let original = r#"{"unexpected\nkey":42}"#;
    fs::write(&file, original).unwrap();
    let error = init(root, false);
    assert!(
        error.contains(r"unknown top-level key unexpected\nkey"),
        "{error:?}"
    );
    assert_eq!(error.lines().count(), 1);
    assert_eq!(fs::read_to_string(file).unwrap(), original);
    assert!(!root.join(HOOK_FILES[0]).exists());
    assert!(!root.join("blackhole.toml").exists());
}

#[test]
fn unrepresentable_names_remain_red() {
    let dir = repo();
    let root = dir.path();
    for name in ["a\\b.py", "a\tb.py", "ordinary.py"] {
        fs::write(root.join(name), "# noqa").unwrap();
    }
    assert!(
        init(root, true)
            .lines()
            .any(|l| l.starts_with("2 finding") && l.contains("blackhole check"))
    );
    assert_eq!(
        fs::read_to_string(root.join(".blackhole/ledger")).unwrap(),
        "blackhole-ledger 1\ncore/no-suppression\t1\tordinary.py\n"
    );
}

// A link to itself cannot be read, so the first ledger fails after the settings are installed.
#[cfg(unix)]
fn fail_first_ledger(root: &Path) {
    std::os::unix::fs::symlink("loop.py", root.join("loop.py")).unwrap();
}

#[cfg(unix)]
#[test]
fn late_init_error_preserves_settings() {
    let dir = repo();
    let root = dir.path();
    fail_first_ledger(root);
    fs::create_dir(root.join(".codex")).unwrap();
    let original = b"{ \"description\": \"Original\" }";
    fs::write(root.join(".codex/hooks.json"), original).unwrap();
    init(root, false);
    assert!(!root.join(".claude/settings.json").exists());
    assert_eq!(fs::read(root.join(".codex/hooks.json")).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn settings_modes_survive_install_and_rollback() {
    use std::os::unix::fs::PermissionsExt;
    for codex_mode in [0o640, 0o660] {
        let files = [
            (".claude/settings.json", 0o600),
            (".codex/hooks.json", codex_mode),
        ];
        for failure in [false, true] {
            let dir = repo();
            let root = dir.path();
            for (file, mode) in files {
                let p = root.join(file);
                fs::create_dir(p.parent().unwrap()).unwrap();
                fs::write(&p, b"{}").unwrap();
                fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
            }
            if failure {
                fail_first_ledger(root);
            }
            let out = common::command("sh")
                .args([
                    "-c",
                    r#"umask 022 && exec "$1" init"#,
                    "init-test",
                    env!("CARGO_BIN_EXE_blackhole"),
                ])
                .current_dir(root)
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(if failure { 2 } else { 1 }));
            for (file, mode) in files {
                let p = root.join(file);
                assert_eq!(
                    fs::metadata(&p).unwrap().permissions().mode() & 0o7777,
                    mode
                );
                if failure {
                    assert_eq!(fs::read(p).unwrap(), b"{}");
                }
            }
            assert_eq!(
                fs::metadata(root.join("blackhole.toml"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o644
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn failed_status_write_leaves_complete_install() {
    let dir = repo();
    let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(dir.path())
        .arg("init")
        .stdout(
            fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(!err.contains("panicked"));
    assert!(err.contains("No space left"));
    assert_eq!(
        fs::read(dir.path().join(".blackhole/ledger")).unwrap(),
        b"blackhole-ledger 1\n"
    );
    let report = init(dir.path(), true);
    for path in [
        ".claude/settings.json",
        ".codex/hooks.json",
        ".blackhole/ledger",
    ] {
        assert!(report.lines().any(|line| line == format!("Kept {path}")));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn failed_settings_write_removes_temporary() {
    let dir = repo();
    let root = dir.path();
    let large = json!({"description":"x".repeat(4096)}).to_string();
    let files = [
        (".claude", "settings.json", "{}"),
        (".codex", "hooks.json", large.as_str()),
    ];
    for (folder, name, bytes) in files {
        fs::create_dir(root.join(folder)).unwrap();
        fs::write(root.join(folder).join(name), bytes).unwrap();
    }
    let out = common::command("sh")
        .current_dir(root)
        .args([
            "-c",
            r#"ulimit -f 2 && trap "" XFSZ && exec "$1" init"#,
            "init-test",
            env!("CARGO_BIN_EXE_blackhole"),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("File too large")
    );
    for (folder, name, bytes) in files {
        assert_eq!(
            fs::read(root.join(folder).join(name)).unwrap(),
            bytes.as_bytes()
        );
        assert!(
            fs::read_dir(root.join(folder)).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn unwritable_stderr_keeps_the_exit_code() {
    let dir = repo();
    let full = || {
        fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap()
    };
    let status = common::command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(dir.path())
        .arg("init")
        .stdout(full())
        .stderr(full())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}

#[test]
fn generated_hooks_skip_the_local_binary_of_another_root() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let dir = repo();
    let root = dir.path();
    init(root, true);
    let outside = tempdir().unwrap();
    common::git(
        root,
        &["config", "core.worktree", outside.path().to_str().unwrap()],
    )
    .unwrap();
    let fake = outside.path().join("node_modules/.bin/blackhole");
    fs::create_dir_all(fake.parent().unwrap()).unwrap();
    fs::write(&fake, "#!/bin/sh\n: >\"$0.ran\"\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let ambient = git_only_path();
    for global in [false, true] {
        if global {
            symlink(
                env!("CARGO_BIN_EXE_blackhole"),
                ambient.path().join("blackhole"),
            )
            .unwrap();
        }
        for name in HOOK_FILES {
            let doc: Value = serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap();
            for (event, input) in [
                (
                    "PreToolUse",
                    json!({"tool_name":"Write","cwd":root,"tool_input":{"file_path":"a.py"}}),
                ),
                ("Stop", json!({})),
            ] {
                let command = doc["hooks"][event][0]["hooks"][0]["command"]
                    .as_str()
                    .unwrap();
                let output = agent::run_input(
                    common::command("/bin/sh")
                        .args(["-c", command])
                        .current_dir(root)
                        .env("PATH", ambient.path()),
                    input.to_string().as_bytes(),
                    2,
                    false,
                );
                let error = String::from_utf8(output.stderr).unwrap();
                assert_eq!(error.lines().count(), 1, "{name} {event}: {error}");
                if global {
                    assert!(error.contains("checkout root mismatch"), "{error}");
                }
                assert!(
                    !outside
                        .path()
                        .join("node_modules/.bin/blackhole.ran")
                        .exists()
                );
            }
        }
    }
}

// Each event ends with init's own guard after the existing groups, which stay as they were.
fn guard_added_beside(existing_group: impl Fn(&str, &Value) -> Option<Value>) {
    let dir = repo();
    let root = dir.path();
    init(root, true);
    for name in HOOK_FILES {
        let p = root.join(name);
        let fresh: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        let mut existing = json!({"hooks":{}});
        for (event, cmd) in [
            ("PreToolUse", "pre-edit"),
            ("PostToolUse", "post-edit"),
            ("Stop", "stop"),
        ] {
            let groups: Vec<Value> = existing_group(cmd, &fresh["hooks"][event][0])
                .into_iter()
                .collect();
            existing["hooks"][event] = json!(groups);
        }
        fs::write(&p, existing.to_string()).unwrap();
        init(root, true);
        let doc: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        for event in ["PreToolUse", "PostToolUse", "Stop"] {
            let mut expected = existing["hooks"][event].as_array().unwrap().clone();
            expected.push(fresh["hooks"][event][0].clone());
            assert_eq!(doc["hooks"][event], json!(expected), "{name} {event}");
        }
        let bytes = fs::read(&p).unwrap();
        init(root, true);
        assert_eq!(fs::read(&p).unwrap(), bytes, "{name} rerun");
    }
}

#[test]
fn guards_with_extra_keys_get_a_plain_one_beside_them() {
    for (key, value) in [
        ("if", json!("Edit(*.ts)")),
        ("args", json!(["hook"])),
        ("async", json!(true)),
        ("asyncRewake", json!(true)),
        ("timeout", json!(30)),
        ("shell", json!("bash")),
    ] {
        for legacy in [false, true] {
            guard_added_beside(|cmd, guard| {
                let mut group = guard.clone();
                group["hooks"][0][key] = value.clone();
                if legacy {
                    group["hooks"][0]["command"] = json!(format!("blackhole hook {cmd}"));
                }
                Some(group)
            });
        }
    }
}

#[test]
fn narrower_guards_get_a_full_one_beside_them() {
    guard_added_beside(|cmd, guard| {
        let mut group = guard.clone();
        group.get("matcher")?;
        group["matcher"] = json!("^Edit$");
        group["hooks"][0]["command"] = json!(previous_launcher(cmd));
        Some(group)
    });
}

#[test]
fn writes_the_judge_workflow_pinned_to_this_version() {
    let dir = repo();
    let root = dir.path();
    let report = init(root, true);
    assert!(
        report
            .lines()
            .any(|line| line == format!("Wrote {WORKFLOW}"))
    );
    let text = fs::read_to_string(root.join(WORKFLOW)).unwrap();
    let version = env!("CARGO_PKG_VERSION");
    assert!(text.contains(&format!(
        "npm install --ignore-scripts --save-exact --no-audit --no-fund @gaspardip/blackhole@{version}\n"
    )));
    assert!(text.contains("\n          npm audit signatures\n"));
    assert!(text.contains(&format!("= \"blackhole {version}\"\n")));
    assert!(text.contains(
        "\non:\n  pull_request_target:\n    types: [opened, reopened, synchronize, edited]\n\npermissions: {}\n"
    ));
    assert!(!text.contains("pull_request:") && !text.contains("pull_request_review"));
    assert_eq!(text.matches("permissions:").count(), 2);
    assert!(text.contains(
        "\n    permissions:\n      contents: read\n      pull-requests: read\n    env:\n"
    ));
    assert!(text.contains("\n  blackhole:\n    name: blackhole\n    runs-on:"));
    // The head is fetched as objects into a bare repository and judged there, never checked out or run.
    assert!(!text.contains("actions/checkout") && !text.contains("checkout -"));
    assert!(text.contains("git init -q --bare \"${repo}\""));
    assert!(text.contains("\"refs/pull/${PR_NUMBER}/head:refs/judge/head\""));
    assert!(text.contains(
        "(cd \"${RUNNER_TEMP}/judge.git\" && \"${RUNNER_TEMP}/blackhole/node_modules/.bin/blackhole\" ci \\\n            --base \"${BASE_SHA}\" --head \"${HEAD_SHA}\" --approvals \"${RUNNER_TEMP}/approvals.json\")"
    ));
    let uses: Vec<_> = text.lines().filter(|line| line.contains("uses:")).collect();
    assert_ne!(uses.len(), 0);
    for line in uses {
        let (_, pin) = line.split_once('@').unwrap();
        let (sha, tag) = pin.split_once(' ').unwrap();
        assert!(
            sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "{line}"
        );
        assert!(tag.starts_with("# v"), "{line}");
    }
}

const PR_COMMITS: &str = "repos/owner/repo/pulls/42/commits?per_page=100";
const PR_REVIEWS: &str = "repos/owner/repo/pulls/42/reviews?per_page=100";
const RETIRED_PERMISSION: &str = "repos/owner/repo/collaborators/Retired/permission";
const GONE_PERMISSION: &str = "repos/owner/repo/collaborators/gone/permission";

fn collect_approvals(runner: &Path, fake: &gh::FakeGh, head: &str) -> std::process::Output {
    let workflow = String::from_utf8(generated_workflow()).unwrap();
    let (_, approvals) = workflow
        .split_once("      - name: Collect approvals of the head\n")
        .unwrap();
    let (_, script) = approvals.split_once("        run: |\n").unwrap();
    let (script, _) = script.split_once("      - name: Judge the head\n").unwrap();
    let script = script
        .lines()
        .map(|line| line.strip_prefix("          ").unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    common::command("bash")
        .args(["-eo", "pipefail", "-c", &script])
        .env("PATH", fake.path())
        .env("RUNNER_TEMP", runner)
        .env("GITHUB_REPOSITORY", "owner/repo")
        .env("PR_NUMBER", "42")
        .env("PR_AUTHOR", "opener")
        .env("HEAD_SHA", head)
        .output()
        .unwrap()
}

fn approval_runner() -> (tempfile::TempDir, String) {
    let runner = tempdir().unwrap();
    let judge = runner.path().join("judge.git");
    fs::create_dir(&judge).unwrap();
    let git = |args: &[&str]| common::git(&judge, args).unwrap();
    git(&["init", "--bare", "-q", "--template="]);
    let tree = git(&["mktree"]);
    let base = git(&["commit-tree", &tree, "-m", "base"]);
    let head = git(&["commit-tree", &tree, "-p", &base, "-m", "head"]);
    git(&["update-ref", "refs/judge/base", &base]);
    git(&["update-ref", "refs/judge/head", &head]);
    (runner, head)
}

fn approval_api(head: &str) -> gh::FakeGh {
    let fake = gh::FakeGh::new();
    let commits = json!([{"sha":head,"author":{"login":"writer"},"committer":null}]);
    fake.respond("GET", PR_COMMITS, 200, &[], commits.to_string().as_bytes());
    let review = |login: &str, state: &str, commit: &str| json!({"user":{"login":login},"state":state,"commit_id":commit});
    let first = json!([
        review("retired", "APPROVED", head),
        review("active", "APPROVED", head),
        review("gone", "APPROVED", head),
        review("old", "APPROVED", "parent"),
    ]);
    let second = json!([
        review("Retired", "DISMISSED", head),
        review("active", "COMMENTED", head),
    ]);
    fake.respond(
        "GET",
        PR_REVIEWS,
        200,
        &[],
        format!("{first}\n{second}").as_bytes(),
    );
    for login in ["retired", "Retired", "gone", "old"] {
        let endpoint = format!("repos/owner/repo/collaborators/{login}/permission");
        fake.respond("GET", &endpoint, 404, &[], b"{}");
    }
    fake.respond(
        "GET",
        "repos/owner/repo/collaborators/active/permission",
        200,
        &[],
        br#"{"permission":"write"}"#,
    );
    fake
}

#[test]
fn approval_collection_checks_only_current_approvers_and_accepts_missing_collaborators() {
    let (runner, head) = approval_runner();
    let fake = approval_api(&head);
    let out = collect_approvals(runner.path(), &fake, &head);
    assert!(out.status.success(), "{out:?}\n{:#?}", fake.calls());
    let approvals: Value =
        serde_json::from_slice(&fs::read(runner.path().join("approvals.json")).unwrap()).unwrap();
    assert_eq!(
        approvals,
        json!({
            "authors":["opener","writer"],
            "reviews":[
                {"login":"active","state":"APPROVED","commit":head,"permission":"write"},
                {"login":"gone","state":"APPROVED","commit":head,"permission":"none"},
            ]
        })
    );
    let calls = fake.calls();
    assert!(
        calls.iter().all(|call| !call.ends_with(RETIRED_PERMISSION)
            && !call.contains("/collaborators/retired/")
            && !call.contains("/collaborators/old/")),
        "{calls:#?}"
    );
    assert!(
        serde_json::from_value::<blackhole_check::verdict::Approvals>(approvals)
            .unwrap()
            .approved(&head)
    );

    fake.respond("GET", GONE_PERMISSION, 502, &[], b"{}");
    let out = collect_approvals(runner.path(), &fake, &head);
    assert!(!out.status.success(), "{out:?}");
}

#[test]
fn approvals_are_ignored_when_the_api_commits_differ_from_the_fetched_commits() {
    let (runner, head) = approval_runner();
    let fake = approval_api(&head);
    fake.respond(
        "GET",
        PR_COMMITS,
        200,
        &[],
        br#"[{"sha":"stale","author":{"login":"writer"}}]"#,
    );
    let out = collect_approvals(runner.path(), &fake, &head);
    assert!(out.status.success(), "{out:?}");
    let approvals: Value =
        serde_json::from_slice(&fs::read(runner.path().join("approvals.json")).unwrap()).unwrap();
    assert_eq!(approvals, json!({"authors":[],"reviews":[]}));
    assert!(
        !serde_json::from_value::<blackhole_check::verdict::Approvals>(approvals)
            .unwrap()
            .approved(&head)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Approvals were ignored because the pull request commit list could not be confirmed.\n"
    );
}

#[test]
fn keeps_an_identical_workflow_and_refuses_a_different_one() {
    let dir = repo();
    let root = dir.path();
    init(root, true);
    let path = root.join(WORKFLOW);
    let generated = fs::read(&path).unwrap();
    assert!(
        init(root, true)
            .lines()
            .any(|line| line == format!("Kept {WORKFLOW}"))
    );
    assert_eq!(fs::read(&path).unwrap(), generated);

    let fresh = repo();
    let root = fresh.path();
    fs::create_dir_all(root.join(".github/workflows")).unwrap();
    let edited = String::from_utf8(generated.clone()).unwrap().replacen(
        "pull_request_target",
        "pull_request",
        1,
    );
    fs::write(root.join(WORKFLOW), &edited).unwrap();
    let message = init(root, false);
    assert_eq!(message.lines().count(), 1);
    assert!(message.contains(WORKFLOW), "{message}");
    assert!(message.contains("first at line 5"), "{message}");
    assert!(message.contains("remove it and rerun init"), "{message}");
    assert_eq!(fs::read_to_string(root.join(WORKFLOW)).unwrap(), edited);
    for name in HOOK_FILES
        .into_iter()
        .chain(["blackhole.toml", ".blackhole/ledger"])
    {
        assert!(!root.join(name).exists(), "{name}");
    }
}

#[test]
fn a_truncated_workflow_reports_the_first_missing_line() {
    let generated = generated_workflow();
    let prefix: Vec<_> = generated
        .split_inclusive(|&byte| byte == b'\n')
        .take(2)
        .flatten()
        .copied()
        .collect();
    let dir = repo();
    let root = dir.path();
    fs::create_dir_all(root.join(".github/workflows")).unwrap();
    fs::write(root.join(WORKFLOW), prefix).unwrap();
    let message = init(root, false);
    assert!(message.contains("first at line 3;"), "{message}");
}

const SCOPES: (&str, &str) = ("X-Oauth-Scopes", "repo, read:org");
const RULES: &str = "repos/owner/repo/rules/branches/main?per_page=100&page=1";
const CLASSIC: &str = "repos/owner/repo/branches/main/protection";
const CONTENTS: &str = "repos/owner/repo/contents/.github/workflows/blackhole.yml?ref=main";

fn generated_workflow() -> Vec<u8> {
    let dir = repo();
    init(dir.path(), true);
    fs::read(dir.path().join(WORKFLOW)).unwrap()
}

fn rules(checks: &Value) -> Vec<u8> {
    json!([
        {"type":"pull_request","ruleset_source_type":"Repository","ruleset_id":7,"parameters":{}},
        {"type":"required_status_checks","ruleset_id":7,"parameters":{"required_status_checks":checks}}
    ])
    .to_string()
    .into_bytes()
}

// Canned answers of a repository whose only ruleset requires pull requests and the blackhole check from GitHub Actions.
fn guarded_github(workflow: &[u8]) -> gh::FakeGh {
    let fake = gh::FakeGh::new();
    fake.respond("GET", "user", 200, &[SCOPES], br#"{"login":"agent"}"#);
    fake.respond(
        "GET",
        "repos/owner/repo",
        200,
        &[],
        br#"{"default_branch":"main","permissions":{"admin":false,"push":true}}"#,
    );
    fake.respond("GET", CONTENTS, 200, &[], workflow);
    let checks = json!([{"context":"blackhole","integration_id":15368}]);
    fake.respond("GET", RULES, 200, &[], &rules(&checks));
    fake.respond(
        "GET",
        "repos/owner/repo/rulesets/7",
        200,
        &[],
        br#"{"id":7,"name":"main rules","current_user_can_bypass":"never"}"#,
    );
    fake.respond("GET", CLASSIC, 404, &[], br#"{"message":"Not Found"}"#);
    fake
}

fn guarded_verdict(fake: &gh::FakeGh, path: Option<std::ffi::OsString>) -> (String, i32) {
    let dir = repo();
    let root = dir.path();
    common::git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/owner/repo.git",
        ],
    )
    .unwrap();
    let out = common::command(env!("CARGO_BIN_EXE_blackhole"))
        .current_dir(root)
        .arg("init")
        .env("PATH", path.unwrap_or_else(|| fake.path()))
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("Wrote blackhole.toml\n"), "{stdout}");
    assert!(stdout.contains(&format!("Wrote {WORKFLOW}\n")), "{stdout}");
    for call in fake.calls() {
        assert!(
            call.starts_with("api --hostname github.com --method GET -i "),
            "{call}"
        );
        assert!(!call.contains(" -f ") && !call.contains(" -F "), "{call}");
    }
    (
        stdout.lines().next_back().unwrap().to_owned(),
        out.status.code().unwrap(),
    )
}

#[test]
fn guarded_repository_reads_as_guarded_and_suggests_the_canary() {
    let fake = guarded_github(&generated_workflow());
    let dir = repo();
    let root = dir.path();
    common::git(
        root,
        &["remote", "add", "origin", "git@github.com:owner/repo.git"],
    )
    .unwrap();
    let out = agent::run_input(
        common::command(env!("CARGO_BIN_EXE_blackhole"))
            .current_dir(root)
            .arg("init")
            .env("PATH", fake.path()),
        b"",
        0,
        false,
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines[lines.len() - 1], "guarded");
    assert!(
        lines[lines.len() - 2].contains("blackhole canary"),
        "{stdout}"
    );
    // Rulesets answered every condition, so the admin-only classic protection was never asked for.
    assert!(!fake.calls().iter().any(|call| call.ends_with(CLASSIC)));
    assert_eq!(fake.calls().len(), 5);
}

enum Canned {
    Respond(
        &'static str,
        u16,
        Vec<(&'static str, &'static str)>,
        Vec<u8>,
    ),
    Fail(&'static str, i32, &'static str),
}
use Canned::{Fail, Respond};

fn assert_verdicts(cases: Vec<(&str, i32, Vec<Canned>)>) {
    let workflow = generated_workflow();
    for (line, code, canned) in cases {
        let fake = guarded_github(&workflow);
        for answer in canned {
            match answer {
                Respond(endpoint, status, headers, body) => {
                    fake.respond("GET", endpoint, status, &headers, &body);
                }
                Fail(endpoint, exit, stderr) => fake.fail("GET", endpoint, exit, stderr),
            }
        }
        assert_eq!(guarded_verdict(&fake, None), (line.to_owned(), code));
    }
}

fn body(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

#[test]
fn unreadable_github_is_not_guarded() {
    assert_verdicts(vec![
        (
            "not guarded: cannot read GitHub (install and sign in to gh)",
            1,
            vec![Fail(
                "user",
                4,
                "To get started with GitHub CLI, please run:  gh auth login\n",
            )],
        ),
        (
            "not guarded: cannot read GitHub (install and sign in to gh)",
            1,
            vec![Respond(
                "user",
                401,
                vec![],
                body(r#"{"message":"Bad credentials"}"#),
            )],
        ),
        (
            "not guarded: cannot read GitHub (error connecting to api.github.com; rerun init once GitHub answers)",
            75,
            vec![Fail(
                "user",
                1,
                "error connecting to api.github.com\ncheck your internet connection\n",
            )],
        ),
        (
            "not guarded: cannot read GitHub (repos/owner/repo/rules/branches/main?per_page=100&page=1 answered HTTP 502; rerun init once GitHub answers)",
            75,
            vec![Respond(RULES, 502, vec![], vec![])],
        ),
        (
            "not guarded: cannot read owner/repo on GitHub (sign gh in as an account that can read it)",
            1,
            vec![Respond(
                "repos/owner/repo",
                404,
                vec![],
                body(r#"{"message":"Not Found"}"#),
            )],
        ),
    ]);
    let missing = git_only_path();
    let fake = guarded_github(&generated_workflow());
    assert_eq!(
        guarded_verdict(&fake, Some(missing.path().into())),
        (
            "not guarded: cannot read GitHub (install and sign in to gh)".to_owned(),
            1
        )
    );
    assert_eq!(fake.calls(), Vec::<String>::new());
}

#[test]
fn a_malformed_rules_list_cannot_establish_a_guard() {
    let workflow = generated_workflow();
    for answer in [b"{}".as_slice(), b"invalid JSON"] {
        let fake = guarded_github(&workflow);
        fake.respond("GET", RULES, 200, &[], answer);
        assert_eq!(
            guarded_verdict(&fake, None),
            (
                format!(
                    "not guarded: cannot read GitHub ({RULES} returned a malformed rules list; rerun init once GitHub answers)"
                ),
                75,
            )
        );
    }
}

#[test]
fn a_later_page_can_require_the_blackhole_check() {
    let fake = guarded_github(&generated_workflow());
    let second = RULES.replace("&page=1", "&page=2");
    let first = json!(vec![json!({"type":"non_fast_forward","ruleset_id":7}); 100]);
    fake.respond("GET", RULES, 200, &[], first.to_string().as_bytes());
    let checks = json!([{"context":"blackhole","integration_id":15368}]);
    fake.respond("GET", &second, 200, &[], &rules(&checks));
    assert_eq!(guarded_verdict(&fake, None), ("guarded".to_owned(), 0));
    assert!(fake.calls().iter().any(|call| call.ends_with(&second)));
}

#[test]
fn a_malformed_rule_on_a_later_page_cannot_establish_a_guard() {
    let fake = guarded_github(&generated_workflow());
    let second = RULES.replace("&page=1", "&page=2");
    let checks = json!([{"context":"blackhole","integration_id":15368}]);
    let mut first: Vec<Value> = serde_json::from_slice(&rules(&checks)).unwrap();
    first.resize(100, json!({"type":"non_fast_forward","ruleset_id":7}));
    fake.respond("GET", RULES, 200, &[], json!(first).to_string().as_bytes());
    fake.respond("GET", &second, 200, &[], b"[null]");
    assert_eq!(
        guarded_verdict(&fake, None),
        (
            format!(
                "not guarded: cannot read GitHub ({second} returned a malformed rules list; rerun init once GitHub answers)"
            ),
            75
        )
    );
}

#[test]
fn a_rule_missing_a_required_field_cannot_establish_a_guard() {
    let fake = guarded_github(&generated_workflow());
    let checks = json!([{"context":"blackhole","integration_id":15368}]);
    let first: Vec<Value> = serde_json::from_slice(&rules(&checks)).unwrap();
    for invalid in [
        json!({"type":"non_fast_forward"}),
        json!({"type":"non_fast_forward","ruleset_id":-1}),
        json!({"ruleset_id":7}),
        json!({"type":"required_status_checks","ruleset_id":7}),
        json!({"type":"required_status_checks","ruleset_id":7,"parameters":{"required_status_checks":[null]}}),
    ] {
        let mut entries = first.clone();
        entries.push(invalid.clone());
        fake.respond(
            "GET",
            RULES,
            200,
            &[],
            json!(entries).to_string().as_bytes(),
        );
        assert_eq!(
            guarded_verdict(&fake, None),
            (
                format!(
                    "not guarded: cannot read GitHub ({RULES} returned a malformed rules list; rerun init once GitHub answers)"
                ),
                75
            ),
            "{invalid}"
        );
    }
}

#[test]
fn each_missing_branch_rule_is_the_one_final_line() {
    let only = |kind: &str| {
        json!([{"type":kind,"ruleset_id":7,"parameters":{"required_status_checks":[{"context":"blackhole","integration_id":15368}]}}])
            .to_string()
            .into_bytes()
    };
    let unprotected = || {
        Respond(
            CLASSIC,
            404,
            vec![],
            body(r#"{"message":"Branch not protected"}"#),
        )
    };
    assert_verdicts(vec![
        (
            "not guarded: .github/workflows/blackhole.yml is not on main (commit it and merge it into main)",
            1,
            vec![Respond(
                CONTENTS,
                404,
                vec![],
                body(r#"{"message":"Not Found"}"#),
            )],
        ),
        (
            "not guarded: .github/workflows/blackhole.yml on main differs from the one init writes (merge the generated file into main)",
            1,
            vec![Respond(CONTENTS, 200, vec![], body("name: blackhole\n"))],
        ),
        (
            "not guarded: main does not require pull requests (add a ruleset for main that requires a pull request before merging)",
            1,
            vec![
                Respond(RULES, 200, vec![], only("required_status_checks")),
                unprotected(),
            ],
        ),
        (
            "not guarded: cannot read the branch protection of main without admin rights (require pull requests and the blackhole check in a ruleset for main, which gh can read)",
            1,
            vec![Respond(RULES, 200, vec![], body("[]"))],
        ),
        (
            "not guarded: main does not require the blackhole check (require the blackhole check from GitHub Actions in a ruleset for main)",
            1,
            vec![
                Respond(RULES, 200, vec![], only("pull_request")),
                unprotected(),
            ],
        ),
        (
            "not guarded: main accepts the blackhole check from a source other than GitHub Actions (set the check's source to GitHub Actions)",
            1,
            vec![
                Respond(RULES, 200, vec![], rules(&json!([{"context":"blackhole"}]))),
                unprotected(),
            ],
        ),
    ]);
}

#[test]
fn each_credential_that_can_get_past_the_judge_is_not_guarded() {
    assert_verdicts(vec![
        (
            "not guarded: gh is signed in as agent, an admin of owner/repo who can bypass or change the rules (sign gh in as an account without admin rights)",
            1,
            // Classic protection alone, which GitHub shows to an admin.
            vec![
                Respond(RULES, 200, vec![], body("[]")),
                Respond(
                    CLASSIC,
                    200,
                    vec![],
                    body(
                        r#"{"required_pull_request_reviews":{},"required_status_checks":{"checks":[{"context":"blackhole","app_id":15368}]}}"#,
                    ),
                ),
                Respond(
                    "repos/owner/repo",
                    200,
                    vec![],
                    body(r#"{"default_branch":"main","permissions":{"admin":true}}"#),
                ),
            ],
        ),
        (
            "not guarded: gh is signed in as agent, who can bypass ruleset main rules (take agent and its teams and roles off the ruleset's bypass list)",
            1,
            vec![Respond(
                "repos/owner/repo/rulesets/7",
                200,
                vec![],
                body(r#"{"name":"main rules","current_user_can_bypass":"pull_requests_only"}"#),
            )],
        ),
        (
            "not guarded: gh's token holds the workflow scope, so it can rewrite the judge (run gh auth refresh --remove-scopes workflow)",
            1,
            vec![Respond(
                "user",
                200,
                vec![("X-Oauth-Scopes", "repo, workflow")],
                body(r#"{"login":"agent"}"#),
            )],
        ),
        (
            "not guarded: GitHub does not reveal the rights of gh's token, so it may rewrite workflows (sign gh in with gh auth login, whose token lists its scopes)",
            1,
            vec![Respond("user", 200, vec![], body(r#"{"login":"agent"}"#))],
        ),
    ]);
}

// Every entry under the root except `.git`, with directories, so an empty directory left behind shows up.
fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, Option<Vec<u8>>)> {
    let mut entries = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path == root.join(".git") {
                continue;
            }
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if path.is_dir() {
                pending.push(path);
                entries.push((relative, None));
            } else {
                entries.push((relative, Some(fs::read(&path).unwrap())));
            }
        }
    }
    entries.sort();
    entries
}

#[test]
fn committing_and_reverting_the_adoption_restores_every_byte() {
    let dir = repo();
    let root = dir.path();
    let numbers = "[1E2, -0.0, 1.50, 12345678901234567890123]";
    for (name, doc) in HOOK_FILES.into_iter().zip([
        format!("{{\n\t\"n\": {numbers},\"s\":\"\\u00e9\"\r\n}}"),
        format!("{{\"description\":\"\\u00e9\",\r\n\t\"hooks\": {{\"Later\": {numbers}}}}}"),
    ]) {
        let path = root.join(name);
        fs::create_dir(path.parent().unwrap()).unwrap();
        fs::write(path, doc).unwrap();
    }
    fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    fs::write(root.join("app.py"), "print(1)\n").unwrap();
    common::git(root, &["add", "-A"]).unwrap();
    common::git(root, &["commit", "-qm", "before"]).unwrap();
    let before = snapshot(root);
    let config = fs::read(root.join(".git/config")).unwrap();
    init(root, true);
    let written = common::git(
        root,
        &[
            "status",
            "--porcelain",
            "--ignored",
            "--untracked-files=all",
        ],
    )
    .unwrap();
    assert_eq!(
        written,
        "M .claude/settings.json\n M .codex/hooks.json\n?? .blackhole/ledger\n?? .github/workflows/blackhole.yml\n?? blackhole.toml"
    );
    common::git(root, &["add", "-A"]).unwrap();
    common::git(root, &["commit", "-qm", "adopt blackhole"]).unwrap();
    common::git(root, &["revert", "--no-edit", "HEAD"]).unwrap();
    assert_eq!(snapshot(root), before);
    assert_eq!(
        common::git(
            root,
            &[
                "status",
                "--porcelain",
                "--ignored",
                "--untracked-files=all"
            ]
        )
        .unwrap(),
        ""
    );
    assert_eq!(fs::read(root.join(".git/config")).unwrap(), config);
}

#[test]
fn a_path_git_ignores_stops_init_before_any_write() {
    for (rule, path) in [
        ("blackhole.toml", "blackhole.toml"),
        (".claude/", ".claude/settings.json"),
        ("*.json", ".claude/settings.json"),
        (".github", WORKFLOW),
        ("/.blackhole/*", ".blackhole/ledger"),
    ] {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join(".gitignore"), format!("{rule}\n")).unwrap();
        let message = init(root, false);
        assert_eq!(
            message,
            format!(
                "blackhole: git ignores {path}, so a commit would leave it out (remove the rule that ignores it, then rerun init)\n"
            ),
            "{rule}"
        );
        assert_eq!(
            snapshot(root),
            [(".gitignore".into(), Some(format!("{rule}\n").into_bytes()))],
            "{rule}"
        );
    }
    // Info/exclude is one of git's ignore sources too.
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".git/info")).unwrap();
    fs::write(root.join(".git/info/exclude"), ".codex\n").unwrap();
    assert!(init(root, false).contains("git ignores .codex/hooks.json"));
}

#[test]
fn an_ignore_rule_does_not_hide_tracked_settings() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join(".claude")).unwrap();
    fs::write(root.join(".claude/settings.json"), "{}").unwrap();
    common::git(root, &["add", "-A"]).unwrap();
    fs::write(root.join(".gitignore"), ".claude/\n").unwrap();
    init(root, true);
}
