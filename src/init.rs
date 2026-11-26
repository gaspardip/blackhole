mod settings_json;

use settings_json::Json;
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
    crate::config::parse(existing.as_deref().unwrap_or_default())?;
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
        let merged = merge(previous.as_deref(), matcher)?;
        Ok::<_, io::Error>(Settings {
            path,
            previous,
            merged,
        })
    };
    read().map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))
}

fn valid_groups(groups: &Json) -> bool {
    groups.as_array().is_some_and(|groups| {
        groups.iter().all(|group| {
            let Some(group) = group.as_object() else {
                return false;
            };
            group
                .get("matcher")
                .is_none_or(|matcher| matcher.as_str().is_some())
                && group
                    .get("hooks")
                    .and_then(Json::as_array)
                    .is_some_and(|hooks| {
                        hooks.iter().all(|hook| {
                            let Some(hook) = hook.as_object() else {
                                return false;
                            };
                            hook.get("type").and_then(Json::as_str).is_some_and(|kind| {
                                kind != "command"
                                    || hook.get("command").and_then(Json::as_str).is_some()
                            })
                        })
                    })
        })
    })
}

fn merge(bytes: Option<&[u8]>, matcher: &str) -> io::Result<Option<Vec<u8>>> {
    let mut doc = match bytes {
        Some(bytes) => serde_json::from_slice::<Json>(bytes)?,
        None => Json::object([]),
    };
    let invalid = || io::Error::other(INVALID_HOOKS);
    let hooks = doc
        .as_object_mut()
        .ok_or_else(invalid)?
        .entry("hooks", || Json::object([]))
        .as_object_mut()
        .ok_or_else(invalid)?;
    if !hooks.values().all(valid_groups) {
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
        let legacy = format!("blackhole hook {cmd}");
        let launcher = |root| {
            format!(
                r#"if root=$({root} 2>/dev/null) && local_bin="$root/node_modules/.bin/blackhole" && [ -f "$local_bin" ] && [ -x "$local_bin" ]; then exec "$local_bin" hook {cmd}; fi; exec blackhole hook {cmd}"#
            )
        };
        let previous = launcher(PREVIOUS_ROOT);
        let command = launcher(ISOLATED_ROOT);
        let mut found = false;
        for group in entries.iter_mut().filter_map(Json::as_object_mut) {
            if group.get("matcher").and_then(Json::as_str) != matcher {
                continue;
            }
            let commands = group
                .entry("hooks", || Json::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(invalid)?;
            for hook in commands.iter_mut().filter_map(Json::as_object_mut) {
                if hook.get("type").and_then(Json::as_str) != Some("command") {
                    continue;
                }
                let installed = hook.entry("command", || Json::string(""));
                if matches!(installed.as_str(), Some(current) if current == legacy || current == previous)
                {
                    *installed = Json::string(&command);
                    changed = true;
                }
                found |= installed.as_str() == Some(&command);
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
    let mut output = serde_json::to_vec_pretty(&doc)?;
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
    let mut scan = crate::scan(root, crate::Targets::Command(&[]), &mut io::sink())?;
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
