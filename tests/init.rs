#[path = "common/agent.rs"]
mod agent;
#[path = "common/cli.rs"]
mod cli;
pub mod common;
use agent::run;
use cli::{check, repo};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::tempdir;

const HOOK_FILES: [&str; 2] = [".claude/settings.json", ".codex/hooks.json"];

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

fn checked_launcher(event: &str) -> String {
    format!(
        r#"if root=$({ISOLATED_ROOT} 2>/dev/null) && top=$(cd -P -- "$root" && pwd -P) && dir=$(pwd -P) && while [ "$dir" != / ] && [ ! -e "$dir/.git" ] && [ ! -L "$dir/.git" ]; do dir=${{dir%/*}}; dir=${{dir:-/}}; done && {{ [ -e "$dir/.git" ] || [ -L "$dir/.git" ]; }} && [ "$dir" = "$top" ] && local_bin="$top/node_modules/.bin/blackhole" && [ -f "$local_bin" ] && [ -x "$local_bin" ]; then exec "$local_bin" hook {event}; fi; command -v blackhole >/dev/null 2>&1 || {{ echo "blackhole: hook {event} needs blackhole on PATH or installed in this checkout" >&2; exit 2; }}; exec blackhole hook {event}"#
    )
}

fn project_launcher(event: &str) -> String {
    let checked = checked_launcher(event);
    let (local, global) = checked.split_once("; fi; command -v blackhole").unwrap();
    format!(
        r#"{local}; fi; if ! command -v git >/dev/null 2>&1 && [ -n "${{CLAUDE_PROJECT_DIR:-}}" ] && local_bin="$CLAUDE_PROJECT_DIR/node_modules/.bin/blackhole" && [ -f "$local_bin" ] && [ -x "$local_bin" ]; then exec "$local_bin" hook {event}; fi; command -v blackhole{global}"#
    )
}

fn status_127_launcher(event: &str) -> String {
    let checked = checked_launcher(event);
    let (local, on_path) = checked.split_once("; fi; command -v blackhole").unwrap();
    let condition = local.split_once("; then exec").unwrap().0;
    let (on_path, _) = on_path.rsplit_once("; exec").unwrap();
    format!(
        r#"{condition}; then hook_bin="$local_bin"; else command -v blackhole{on_path}; hook_bin=blackhole; fi; hook_status=0; (exec "$hook_bin" hook {event}) || hook_status=$?; case "$hook_status" in 126|127) echo "blackhole: hook {event} could not run blackhole (exit $hook_status)" >&2; exit 2;; esac; exit "$hook_status""#
    )
}

fn init(root: &Path, success: bool) -> String {
    let out = run(root, &["init"], "", if success { 0 } else { 2 }, false);
    // A run that fails reports no file as written.
    assert!(success || out.stdout.is_empty(), "{out:?}");
    String::from_utf8(if success { out.stdout } else { out.stderr }).unwrap()
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
                    {"type":"command","command":checked_launcher("stop")},
                    {"type":"command","command":project_launcher("stop")},
                    {"type":"command","command":status_127_launcher("stop")},
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
    assert!(init(root, true).contains("CI"));
    assert_eq!(
        fs::read(root.join("blackhole.toml")).unwrap(),
        b"exclude=[]"
    );
    assert_eq!(
        fs::read(root.join(".blackhole/ledger")).unwrap(),
        b"blackhole-ledger 1\n"
    );
    let mut installed = None;
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
        assert_ne!(stop, &Value::String(checked_launcher("stop")));
        assert_ne!(stop, &Value::String(project_launcher("stop")));
        if let Some(previous) = installed.replace(stop.clone()) {
            assert_eq!(stop, &previous);
        }
        let mut upgraded = original.clone();
        for index in 0..6 {
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
            assert_eq!(out.status.success(), !failure);
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
fn the_launcher_uses_only_path_without_git() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = repo();
    let root = dir.path();
    fs::write(root.join(".gitignore"), "/node_modules/\n").unwrap();
    init(root, true);
    let local = root.join("node_modules/.bin/blackhole");
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    symlink(env!("CARGO_BIN_EXE_blackhole"), &local).unwrap();
    // Without git a stray marker must not pick the binary beside it.
    let stray = root.join("stray");
    let fake = stray.join("node_modules/.bin/blackhole");
    fs::create_dir_all(fake.parent().unwrap()).unwrap();
    fs::write(stray.join(".git"), "").unwrap();
    fs::write(&fake, "#!/bin/sh\n: >\"$0.ran\"\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let empty = tempdir().unwrap();
    let global = tempdir().unwrap();
    symlink(
        env!("CARGO_BIN_EXE_blackhole"),
        global.path().join("blackhole"),
    )
    .unwrap();
    for name in HOOK_FILES {
        let doc: Value = serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap();
        let command = doc["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        for cwd in [root, &stray] {
            for (path, active, status, expected) in [
                (
                    empty.path(),
                    true,
                    2,
                    "blackhole: hook stop needs blackhole on PATH",
                ),
                (
                    empty.path(),
                    false,
                    2,
                    "blackhole: hook stop needs blackhole on PATH",
                ),
                (
                    global.path(),
                    true,
                    0,
                    "blackhole: no verdict: cannot run git: ",
                ),
                (
                    global.path(),
                    false,
                    2,
                    "blackhole: no verdict: cannot run git: ",
                ),
            ] {
                let output = agent::run_input(
                    common::command("/bin/sh")
                        .args(["-c", command])
                        .current_dir(cwd)
                        .env("PATH", path)
                        .env("CLAUDE_PROJECT_DIR", root),
                    json!({"stop_hook_active": active}).to_string().as_bytes(),
                    status,
                    false,
                );
                // A released stop shows its warning as a systemMessage, since hosts drop stderr on exit 0.
                let shown = if status == 0 {
                    let message: Value = serde_json::from_slice(&output.stdout).unwrap();
                    message["systemMessage"]
                        .as_str()
                        .unwrap()
                        .as_bytes()
                        .to_vec()
                } else {
                    output.stderr.clone()
                };
                assert!(
                    shown.starts_with(expected.as_bytes()),
                    "{name} {cwd:?} {active}: {output:?}"
                );
            }
        }
    }
    assert!(!stray.join("node_modules/.bin/blackhole.ran").exists());
}

#[test]
fn generated_stop_blocks_when_the_selected_binary_cannot_start() {
    use std::os::unix::fs::PermissionsExt;

    let dir = repo();
    let root = dir.path();
    init(root, true);
    let ambient = git_only_path();
    let local = root.join("node_modules/.bin/blackhole");
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    for bin in [local, ambient.path().join("blackhole")] {
        for (script, status, launch_failed) in [
            ("#!/no/such/blackhole-interpreter\n", 2, true),
            ("#!/bin/sh\nexit 126\n", 2, true),
            ("#!/bin/sh\nexit 127\n", 2, true),
            ("", 2, true),
            ("#!/bin/sh\nexit 1\n", 2, true),
            ("#!/bin/sh\nulimit -c 0; kill -ABRT $$\n", 2, true),
            ("#!/bin/sh\nexit 0\n", 0, false),
            ("#!/bin/sh\nexit 2\n", 2, false),
            ("#!/bin/sh\nexit 75\n", 75, false),
        ] {
            fs::write(&bin, script).unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
            for name in HOOK_FILES {
                let doc: Value =
                    serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap();
                let command = doc["hooks"]["Stop"][0]["hooks"][0]["command"]
                    .as_str()
                    .unwrap();
                let output = common::command("/bin/sh")
                    .args(["-c", &format!("blackhole() {{ exit 0; }}; {command}")])
                    .current_dir(root)
                    .env("PATH", ambient.path())
                    .output()
                    .unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(status),
                    "{name} {bin:?}: {output:?}"
                );
                assert_eq!(
                    String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .filter(
                            |line| line.starts_with("blackhole: hook stop could not run blackhole")
                        )
                        .count(),
                    usize::from(launch_failed),
                    "{name} {bin:?}: {output:?}"
                );
            }
        }
        fs::remove_file(bin).unwrap();
    }
}
