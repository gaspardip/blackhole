mod settings_json;

use settings_json::{Json, Object};
use std::{
    io::{self, Write},
    path::Path,
};

const INVALID_HOOKS: &str = "unexpected hooks configuration";
const CLAUDE: &str = ".claude/settings.json";
const CODEX: &str = ".codex/hooks.json";
const PREVIOUS_ROOT: &str = "git --no-lazy-fetch rev-parse --show-toplevel";
// Match the CLI's Git environment without changing the environment of the hook itself.
const ISOLATED_ROOT: &str = r#"names=$(command -p env | LC_ALL=C command -p sed -n 's/^\(GIT_[A-Za-z0-9_]*\)=.*/\1/p') || exit
while IFS= read -r name; do
    [ -z "$name" ] || unset "$name" || exit
done <<BLACKHOLE_GIT_ENV
$names
BLACKHOLE_GIT_ENV
exec git --no-replace-objects --no-lazy-fetch rev-parse --show-toplevel"#;

/// Installs project hooks and creates the initial ledger.
///
/// # Errors
/// Reports invalid settings, repository discovery and filesystem failures.
pub fn run(output: &mut impl Write) -> io::Result<()> {
    let root = crate::files::root()?;
    let _lock = crate::files::lock(&root)?;
    let claude = settings(&root, CLAUDE, "^(Edit|Write|MultiEdit|NotebookEdit)$")?;
    let codex = settings(&root, CODEX, "^apply_patch$")?;
    let config = root.join("blackhole.toml");
    let existing = crate::files::policy(&config)?;
    // A malformed ledger or objections file would turn every hook red, so it stops init before anything is written.
    let policy = crate::Policy::load(&root)?;
    crate::objections::parse(
        &crate::policy(policy.entered, "objections")?.unwrap_or_default(),
        crate::objections::Date::today()?,
    )?;
    install(&root, "blackhole.toml", existing.is_none().then_some(&[]))?;
    let created =
        claude.install_then(&root, || codex.install_then(&root, || first_ledger(&root)))?;
    let mut report = Vec::new();
    for (path, wrote) in [
        ("blackhole.toml", existing.is_none()),
        (CLAUDE, claude.merged.is_some()),
        (CODEX, codex.merged.is_some()),
    ] {
        writeln!(report, "{} {path}", if wrote { "Wrote" } else { "Kept" })?;
    }
    writeln!(
        report,
        "{} {}",
        if created.is_some() { "Created" } else { "Kept" },
        crate::files::LEDGER_PATH
    )?;
    if let Some(count @ 1..) = created {
        writeln!(
            report,
            "{count} finding(s) remain red. Run blackhole check to list them."
        )?;
    }
    writeln!(
        report,
        "CI is not set up yet. Hooks give early warning. Shell edits bypass edit hooks. blackhole ci is the judge."
    )?;
    writeln!(
        report,
        "The judge holds only when the agent has weaker credentials than a person.\nKeep admin, bypass, workflow permissions and the approving account out of the agent's reach."
    )?;
    output.write_all(&report)
}

fn parts(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or((".", path))
}

struct Settings<'a> {
    path: &'a str,
    previous: Option<Vec<u8>>,
    merged: Option<Vec<u8>>,
}

impl Settings<'_> {
    fn install_then<T>(&self, root: &Path, next: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        install(root, self.path, self.merged.as_deref())?;
        let result = next();
        if let Err(error) = &result
            && self.merged.is_some()
        {
            // The first ledger must see the installed settings, which can themselves have findings.
            let (dir, name) = parts(self.path);
            let restore = || {
                crate::enter(root, dir, true)?;
                match &self.previous {
                    Some(bytes) => crate::replace(name, bytes),
                    None => std::fs::remove_file(name),
                }
            };
            restore()
                .map_err(|e| io::Error::other(format!("{error}; restoring {}: {e}", self.path)))?;
        }
        result
    }
}

fn settings<'a>(root: &Path, path: &'a str, matcher: &str) -> io::Result<Settings<'a>> {
    let (dir, name) = parts(path);
    let read = || {
        let entered = crate::enter(root, dir, false)?;
        let previous = crate::policy(entered, name)?;
        let merged = merge(previous.as_deref(), matcher, path == CODEX)?;
        Ok::<_, io::Error>(Settings {
            path,
            previous,
            merged,
        })
    };
    read().map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))
}

fn optional(value: Option<&Json>, valid: impl FnOnce(&Json) -> bool) -> bool {
    value.is_none_or(|value| value.is_null() || valid(value))
}

fn string(value: &Json) -> bool {
    value.as_str().is_some()
}

fn valid_hook(hook: &Json, codex: bool) -> bool {
    let Some(hook) = hook.as_object() else {
        return false;
    };
    let kind = hook.get("type").and_then(Json::as_str);
    if !codex {
        return kind
            .is_some_and(|kind| kind != "command" || hook.get("command").is_some_and(string));
    }
    let valid_timeout = || optional(hook.get("timeout"), |value| value.as_u64().is_some());
    let valid_status = || optional(hook.get("statusMessage"), string);
    match kind {
        Some("command") => {
            hook.get("command").is_some_and(string)
                && optional(
                    hook.get("commandWindows")
                        .or_else(|| hook.get("command_windows")),
                    string,
                )
                && !(hook.get("commandWindows").is_some() && hook.get("command_windows").is_some())
                && valid_timeout()
                && valid_status()
                && hook.get("async").is_none_or(Json::is_bool)
                && optional(hook.get("additionalContextLimit"), |value| {
                    value.as_u64().is_some_and(|n| usize::try_from(n).is_ok())
                })
        }
        Some("mcp_tool") => {
            hook.get("server").is_some_and(string)
                && hook.get("tool").is_some_and(string)
                && hook
                    .get("input")
                    .is_none_or(|value| value.as_object().is_some() && value.is_toml_value())
                && valid_timeout()
                && valid_status()
        }
        Some("prompt" | "agent") => true,
        _ => false,
    }
}

fn valid_groups(groups: &Json, codex: bool) -> bool {
    groups.as_array().is_some_and(|groups| {
        groups.iter().all(|group| {
            let Some(group) = group.as_object() else {
                return false;
            };
            let matcher = group.get("matcher");
            let valid_matcher = if codex {
                optional(matcher, string)
            } else {
                matcher.is_none_or(string)
            };
            valid_matcher
                && match group.get("hooks") {
                    None => codex,
                    Some(hooks) => hooks
                        .as_array()
                        .is_some_and(|hooks| hooks.iter().all(|hook| valid_hook(hook, codex))),
                }
        })
    })
}

fn codex_keys(doc: &Object) -> io::Result<()> {
    for (key, value) in doc.entries() {
        match key {
            "hooks" => {}
            "description" if value.is_null() || string(value) => {}
            "description" => return Err(io::Error::other(INVALID_HOOKS)),
            _ => {
                let key = crate::diagnostic_text(key);
                return Err(io::Error::other(format!("unknown top-level key {key}")));
            }
        }
    }
    Ok(())
}

// Codex validates known events and ignores unrecognized entries in the event map.
fn codex_event(event: &str) -> bool {
    matches!(
        event,
        "PreModelRequest"
            | "PostModelRequest"
            | "PreToolUse"
            | "PermissionRequest"
            | "PostToolUse"
            | "PreCompact"
            | "PostCompact"
            | "SessionStart"
            | "SessionEnd"
            | "UserPromptSubmit"
            | "SubagentStart"
            | "SubagentStop"
            | "Stop"
            | "Interrupt"
    )
}

fn merge(bytes: Option<&[u8]>, matcher: &str, codex: bool) -> io::Result<Option<Vec<u8>>> {
    let mut doc = match bytes {
        Some(bytes) => Json::parse(bytes)?,
        None => Json::object([]),
    };
    let invalid = || io::Error::other(INVALID_HOOKS);
    let doc_object = doc.as_object_mut().ok_or_else(invalid)?;
    if codex {
        codex_keys(doc_object)?;
    }
    let hooks = doc_object
        .entry("hooks", || Json::object([]))
        .as_object_mut()
        .ok_or_else(invalid)?;
    if !hooks
        .entries()
        .all(|(event, groups)| (codex && !codex_event(event)) || valid_groups(groups, codex))
    {
        return Err(invalid());
    }
    let mut changed = false;
    for (event, cmd) in [
        ("PreToolUse", "pre-edit"),
        ("PostToolUse", "post-edit"),
        ("Stop", "stop"),
    ] {
        let entries = hooks
            .entry(event, || Json::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(invalid)?;
        let matcher = (event != "Stop").then_some(matcher);
        let launcher = |root| {
            format!(
                r#"if root=$({root} 2>/dev/null) && local_bin="$root/node_modules/.bin/blackhole" && [ -f "$local_bin" ] && [ -x "$local_bin" ]; then exec "$local_bin" hook {cmd}; fi; exec blackhole hook {cmd}"#
            )
        };
        let on_path = format!(
            r#"command -v blackhole >/dev/null 2>&1 || {{ echo "blackhole: hook {cmd} needs blackhole on PATH or installed in this checkout" >&2; exit 2; }}"#
        );
        let global = format!("{on_path}; exec blackhole hook {cmd}");
        let nearest = r#"dir=$(pwd -P) && while [ "$dir" != / ] && [ ! -e "$dir/.git" ] && [ ! -L "$dir/.git" ]; do dir=${dir%/*}; dir=${dir:-/}; done && { [ -e "$dir/.git" ] || [ -L "$dir/.git" ]; }"#;
        let top =
            format!(r#"root=$({ISOLATED_ROOT} 2>/dev/null) && top=$(cd -P -- "$root" && pwd -P)"#);
        // `core.worktree` can point git elsewhere, so the local binary must sit beside the nearest `.git`.
        let local = format!(
            r#"{top} && {nearest} && [ "$dir" = "$top" ] && local_bin="$top/node_modules/.bin/blackhole" && [ -f "$local_bin" ] && [ -x "$local_bin" ]"#
        );
        let checked = format!(r#"if {local}; then exec "$local_bin" hook {cmd}; fi; "#);
        let project = format!(
            r#"if ! command -v git >/dev/null 2>&1 && [ -n "${{CLAUDE_PROJECT_DIR:-}}" ] && local_bin="$CLAUDE_PROJECT_DIR/node_modules/.bin/blackhole" && [ -f "$local_bin" ] && [ -x "$local_bin" ]; then exec "$local_bin" hook {cmd}; fi; "#
        );
        let previous_status = format!(
            r#"if {local}; then hook_bin="$local_bin"; else {on_path}; hook_bin=blackhole; fi; hook_status=0; (exec "$hook_bin" hook {cmd}) || hook_status=$?; case "$hook_status" in 126|127) echo "blackhole: hook {cmd} could not run blackhole (exit $hook_status)" >&2; exit 2;; esac; exit "$hook_status""#
        );
        let command = format!(
            r#"if {local}; then hook_bin="$local_bin"; else hook_bin=$(unset -f blackhole; command -v blackhole) || {{ echo "blackhole: hook {cmd} needs blackhole on PATH or installed in this checkout" >&2; exit 2; }}; fi; [ -s "$hook_bin" ] || {{ echo "blackhole: hook {cmd} could not run blackhole (empty executable)" >&2; exit 2; }}; hook_status=0; {{ (exec "$hook_bin" hook {cmd} 2>&3) || hook_status=$?; }} 3>&2 2>/dev/null; case "$hook_status" in 0|2|75) exit "$hook_status";; *) echo "blackhole: hook {cmd} could not run blackhole (exit $hook_status)" >&2; exit 2;; esac"#
        );
        let known = [
            format!("blackhole hook {cmd}"),
            launcher(PREVIOUS_ROOT),
            launcher(ISOLATED_ROOT),
            format!("{checked}{global}"),
            format!("{checked}{project}{global}"),
            previous_status,
            format!(
                r#"if {nearest} && local_bin="$dir/node_modules/.bin/blackhole" && [ -f "$local_bin" ] && [ -x "$local_bin" ] && {{ ! command -v git >/dev/null 2>&1 || {{ {top} && [ "$dir" = "$top" ]; }}; }}; then exec "$local_bin" hook {cmd}; fi; {global}"#
            ),
        ];
        let mut found = false;
        for group in entries.iter_mut().filter_map(Json::as_object_mut) {
            if group.get("matcher").and_then(Json::as_str) != matcher {
                continue;
            }
            let Some(commands) = group.get_mut("hooks").and_then(Json::as_array_mut) else {
                continue;
            };
            for hook in commands.iter_mut().filter_map(Json::as_object_mut) {
                // Any other key (`if`, `args`, `async`, `timeout`, ...) can keep a hook from blocking.
                if hook
                    .entries()
                    .any(|(key, _)| key != "type" && key != "command")
                    || hook.get("type").and_then(Json::as_str) != Some("command")
                {
                    continue;
                }
                let Some(installed) = hook.get_mut("command").filter(|installed| {
                    installed.as_str().is_some_and(|current| {
                        current == command || known.iter().any(|old| old == current)
                    })
                }) else {
                    continue;
                };
                if installed.as_str() != Some(command.as_str()) {
                    *installed = Json::string(&command);
                    changed = true;
                }
                found = true;
            }
        }
        if !found {
            let hook = Json::object([
                ("type", Json::string("command")),
                ("command", Json::string(&command)),
            ]);
            let mut group = Json::object([("hooks", Json::Array(vec![hook]))]);
            if let Some(matcher) = matcher {
                group
                    .as_object_mut()
                    .ok_or_else(invalid)?
                    .entry("matcher", || Json::string(matcher));
            }
            entries.push(group);
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    let mut output = doc.to_vec_pretty()?;
    output.push(b'\n');
    Ok(Some(output))
}

fn install(root: &Path, path: &str, data: Option<&[u8]>) -> io::Result<()> {
    let Some(data) = data else {
        return Ok(());
    };
    let (dir, name) = parts(path);
    crate::enter(root, dir, true)?;
    crate::replace(name, data)
}

fn first_ledger(root: &Path) -> io::Result<Option<usize>> {
    crate::enter(root, ".blackhole", true)?;
    if crate::policy(true, "ledger")?.is_some() {
        return Ok(None);
    }
    let policy = crate::Policy::load(root)?;
    let mut scan = crate::scan_all(root, &policy, &mut io::sink())?;
    let mut remaining = 0;
    scan.actual.retain(|(rule, path), count| {
        let allowed = crate::engine::can_allow(crate::engine::registry(), rule)
            && !crate::ledger::invalid_path(path);
        if !allowed {
            remaining += *count;
        }
        allowed
    });
    crate::replace("ledger", crate::ledger::write(&scan.actual)?)?;
    Ok(Some(remaining))
}
