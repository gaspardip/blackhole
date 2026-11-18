use crate::verdict::Change;
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Read, Write},
    process::{Child, ChildStdout, Command, Output, Stdio},
};

// The caller's git variables must not point at another repository or index, and a replace ref must not swap an object.
pub(crate) fn command() -> Command {
    let mut command = Command::new("git");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(name);
    }
    command.arg("--no-replace-objects");
    command
}
fn failure(out: &Output) -> io::Error {
    io::Error::other(
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .next()
            .unwrap_or("git failed")
            .to_owned(),
    )
}
pub(crate) fn output(command: &mut Command) -> io::Result<Vec<u8>> {
    let out = command.output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(failure(&out))
    }
}
fn run(args: &[&str]) -> io::Result<String> {
    String::from_utf8(output(command().args(args))?).map_err(io::Error::other)
}
pub(crate) fn commit(rev: &str) -> io::Result<String> {
    let name = run(&[
        "rev-parse",
        "--verify",
        "--end-of-options",
        &format!("{rev}^{{commit}}"),
    ])?;
    Ok(name.trim_end().into())
}
pub(crate) fn ancestor(base: &str, head: &str) -> io::Result<bool> {
    let out = command()
        .args(["merge-base", "--is-ancestor", base, head])
        .output()?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(failure(&out)),
    }
}

enum Entry {
    File(String),
    Link(String),
    Other,
}
pub(crate) struct Tree(BTreeMap<String, Entry>);
impl Tree {
    pub(crate) fn load(sha: &str) -> io::Result<Self> {
        let mut entries = BTreeMap::new();
        let listing = run(&["ls-tree", "--full-tree", "-r", "-t", "-z", sha])?;
        for line in listing.split_terminator('\0') {
            let (header, path) = line
                .split_once('\t')
                .ok_or_else(|| io::Error::other("invalid git tree"))?;
            let parts: Vec<_> = header.split(' ').collect();
            let &[mode, kind, id] = parts.as_slice() else {
                return Err(io::Error::other("invalid tree entry"));
            };
            let entry = match (kind, mode) {
                ("blob", "100644" | "100755") => Entry::File(id.into()),
                ("blob", "120000") => Entry::Link(id.into()),
                _ => Entry::Other,
            };
            entries.insert(path.into(), entry);
        }
        Ok(Self(entries))
    }
    pub(crate) fn judge_file(&self, path: &str, batch: &mut Batch) -> io::Result<Option<Vec<u8>>> {
        match self.0.get(path) {
            None => Ok(None),
            Some(Entry::File(id)) => batch.read(id).map(Some),
            Some(_) => Err(io::Error::other(format!("{path} must be a regular blob"))),
        }
    }
    pub(crate) fn paths(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }
    // A link to a file is read under the link's name, as a checkout reads it. A directory has nothing to read.
    pub(crate) fn read(&self, path: &str, batch: &mut Batch) -> io::Result<Option<Vec<u8>>> {
        let id = match self.0.get(path) {
            Some(Entry::File(id)) => Some(id.as_str()),
            Some(Entry::Link(_)) => self.follow(path, batch)?,
            _ => None,
        };
        id.map(|id| batch.read(id)).transpose()
    }
    // A target the commit does not hold could hide a file from the judge, so it is an error.
    fn follow(&self, link: &str, batch: &mut Batch) -> io::Result<Option<&str>> {
        let lost = || io::Error::other("link target is not in the commit");
        let mut pending: Vec<String> = link.rsplit('/').map(str::to_owned).collect();
        let mut done: Vec<String> = Vec::new();
        let mut hops = 0;
        while let Some(part) = pending.pop() {
            match part.as_str() {
                "" | "." => continue,
                ".." => {
                    done.pop().ok_or_else(lost)?;
                    continue;
                }
                _ => done.push(part),
            }
            match (self.0.get(&done.join("/")), pending.is_empty()) {
                (Some(Entry::Link(id)), _) => {
                    hops += 1;
                    let target = String::from_utf8(batch.read(id)?).map_err(|_| lost())?;
                    // Linux gives up after forty links too.
                    if hops > 40 || target.starts_with('/') {
                        return Err(lost());
                    }
                    done.pop();
                    pending.extend(target.rsplit('/').map(str::to_owned));
                }
                (Some(Entry::File(id)), true) => return Ok(Some(id)),
                (Some(Entry::Other), true) => return Ok(None),
                (Some(Entry::Other), false) => {}
                _ => return Err(lost()),
            }
        }
        Ok(None)
    }
}

pub(crate) fn changes(base: &str, head: &str) -> io::Result<Vec<Change>> {
    let raw = run(&[
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--name-status",
        "-M",
        "-z",
        base,
        head,
        "--",
    ])?;
    let mut fields = raw.split_terminator('\0');
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        let mut path = || {
            fields
                .next()
                .map(str::to_owned)
                .ok_or_else(|| io::Error::other("incomplete git diff"))
        };
        let old = status.starts_with('R').then(&mut path).transpose()?;
        changes.push(Change { old, path: path()? });
    }
    Ok(changes)
}

pub(crate) struct Batch {
    child: Child,
    reader: BufReader<ChildStdout>,
}
impl Batch {
    pub(crate) fn start() -> io::Result<Self> {
        let mut child = command()
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let reader = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("missing git stdout"))?,
        );
        Ok(Self { child, reader })
    }
    pub(crate) fn read(&mut self, id: &str) -> io::Result<Vec<u8>> {
        let input = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("missing git stdin"))?;
        writeln!(input, "{id}")?;
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let header: Vec<_> = line.split_ascii_whitespace().collect();
        let &[oid, "blob", size] = header.as_slice() else {
            return Err(io::Error::other("missing or invalid git blob"));
        };
        if oid != id {
            return Err(io::Error::other("unexpected git blob"));
        }
        let mut bytes = vec![0; size.parse().map_err(io::Error::other)?];
        self.reader.read_exact(&mut bytes)?;
        let mut nl = [0];
        self.reader.read_exact(&mut nl)?;
        if nl != *b"\n" {
            return Err(io::Error::other("invalid git blob terminator"));
        }
        Ok(bytes)
    }
    pub(crate) fn finish(mut self) -> io::Result<()> {
        if self.child.wait()?.success() {
            Ok(())
        } else {
            Err(io::Error::other("git cat-file failed"))
        }
    }
}
impl Drop for Batch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
