use std::{fs, io, path::Path, process::Command};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
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

fn copy(src: &Path, dest: &Path) -> io::Result<()> {
    for entry in fs::read_dir(src)? {
        let e = entry?;
        let p = dest.join(e.file_name());
        if e.file_type()?.is_dir() {
            fs::create_dir(&p)?;
            copy(&e.path(), &p)?;
        } else if e.file_name() != ".keep" {
            fs::copy(e.path(), p)?;
        }
    }
    Ok(())
}

pub fn commit(case: &Path, root: &Path, side: &str) -> Result<String> {
    copy(&case.join(side), root)?;
    git(root, &["add", "-A"])?;
    git(root, &["commit", "--allow-empty", "-qm", side])?;
    git(root, &["rev-parse", "HEAD"])
}
