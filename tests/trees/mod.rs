use crate::common::{Result, git};
use std::{fs, io, path::Path};

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

fn commit(case: &Path, root: &Path, side: &str) -> Result<String> {
    copy(&case.join(side), root)?;
    git(root, &["add", "-A"])?;
    git(root, &["commit", "--allow-empty", "-qm", side])?;
    git(root, &["rev-parse", "HEAD"])
}

pub fn revisions(case: &Path, root: &Path) -> Result<(String, String)> {
    git(root, &["init", "-q"])?;
    let base = commit(case, root, "base")?;
    git(root, &["rm", "-rq", "--ignore-unmatch", "."])?;
    let head = commit(case, root, "head")?;
    Ok((base, head))
}
