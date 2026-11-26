use std::{env, path::Path, process::Command};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// A hook or `git rebase --exec` exports git variables, and with them a fixture's commands would write to the caller's repository.
#[must_use]
pub fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    for (name, _) in env::vars_os() {
        if name.as_encoded_bytes().starts_with(b"GIT_") {
            command.env_remove(name);
        }
    }
    command
}

/// # Errors
/// Returns command, status and decoding errors.
pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = command("git")
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
        .output()?;
    if !out.status.success() {
        return Err(String::from_utf8(out.stderr)?.into());
    }
    Ok(String::from_utf8(out.stdout)?.trim().into())
}
