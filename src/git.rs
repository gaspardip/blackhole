use crate::verdict::{Change, Kind};
use std::{
    collections::BTreeMap,
    env,
    io::{self, BufRead, BufReader, Read, Write},
    process::{Child, ChildStdout, Command, Output, Stdio},
};

// The caller's git variables must not point at another repository or index, and a replace ref must not swap an object.
pub(crate) fn command() -> Command {
    let mut command = Command::new("git");
    for (name, _) in env::vars_os() {
        if name.as_encoded_bytes().starts_with(b"GIT_") {
            command.env_remove(name);
        }
    }
    command.args([
        "--no-replace-objects",
        // The checkout's configuration names a program for each of these, and a call this crate makes would run it.
        "--no-lazy-fetch",
        "-c",
        "core.fsmonitor=false",
    ]);
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
pub(crate) fn repository_root() -> io::Result<std::path::PathBuf> {
    if run(&["rev-parse", "--is-bare-repository"])? == "true\n" {
        let git_dir = run(&["rev-parse", "--absolute-git-dir"])?;
        std::fs::canonicalize(git_dir.strip_suffix('\n').unwrap_or(&git_dir))
    } else {
        crate::files::root()
    }
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
enum Blob<'a> {
    File(&'a str),
    Absent,
    UnresolvedLink,
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
    pub(crate) fn repository_paths(
        &self,
        batch: &mut Batch,
    ) -> io::Result<crate::packs::boundary::RepositoryPaths> {
        let mut paths = crate::packs::boundary::RepositoryPaths::default();
        for (name, entry) in &self.0 {
            let path = std::path::Path::new("/").join(name);
            match entry {
                Entry::File(_) => {
                    paths.files.insert(path);
                }
                Entry::Link(id) => {
                    let target = String::from_utf8(batch.read(id)?).map_err(io::Error::other)?;
                    paths.links.insert(path, target.into());
                }
                Entry::Other => (),
            }
        }
        Ok(paths)
    }

    // A link to a file is read under the link's name, as a checkout reads it. A directory has nothing to read.
    pub(crate) fn read(&self, path: &str, batch: &mut Batch) -> io::Result<Option<Vec<u8>>> {
        self.blob(path, batch)?.map(|id| batch.read(id)).transpose()
    }
    pub(crate) fn blob(&self, path: &str, batch: &mut Batch) -> io::Result<Option<&str>> {
        match self.resolve_blob(path, batch)? {
            Blob::File(id) => Ok(Some(id)),
            Blob::Absent => Ok(None),
            Blob::UnresolvedLink => Err(io::Error::other("link target is not in the commit")),
        }
    }

    // A base link without a target has no inventory to compare.
    pub(crate) fn historical_blob(
        &self,
        path: &str,
        batch: &mut Batch,
    ) -> io::Result<Option<&str>> {
        match self.resolve_blob(path, batch)? {
            Blob::File(id) => Ok(Some(id)),
            Blob::Absent | Blob::UnresolvedLink => Ok(None),
        }
    }

    fn resolve_blob(&self, path: &str, batch: &mut Batch) -> io::Result<Blob<'_>> {
        match self.0.get(path) {
            Some(Entry::File(id)) => Ok(Blob::File(id)),
            Some(Entry::Link(_)) => self.follow(path, batch),
            _ => Ok(Blob::Absent),
        }
    }

    fn follow(&self, link: &str, batch: &mut Batch) -> io::Result<Blob<'_>> {
        let mut pending: Vec<String> = link.rsplit('/').map(str::to_owned).collect();
        let mut done: Vec<String> = Vec::new();
        let mut hops = 0;
        while let Some(part) = pending.pop() {
            match part.as_str() {
                "" | "." => continue,
                ".." => {
                    if done.pop().is_none() {
                        return Ok(Blob::UnresolvedLink);
                    }
                    continue;
                }
                _ => done.push(part),
            }
            match (self.0.get(&done.join("/")), pending.is_empty()) {
                (Some(Entry::Link(id)), _) => {
                    hops += 1;
                    let Ok(target) = String::from_utf8(batch.read(id)?) else {
                        return Ok(Blob::UnresolvedLink);
                    };
                    // Linux gives up after forty links too.
                    if hops > 40 || target.starts_with('/') {
                        return Ok(Blob::UnresolvedLink);
                    }
                    done.pop();
                    pending.extend(target.rsplit('/').map(str::to_owned));
                }
                (Some(Entry::File(id)), true) => return Ok(Blob::File(id)),
                (Some(Entry::Other), true) => return Ok(Blob::Absent),
                (Some(Entry::Other), false) => {}
                _ => return Ok(Blob::UnresolvedLink),
            }
        }
        Ok(Blob::Absent)
    }
}

pub(crate) fn changes(base: &str, head: &str) -> io::Result<Vec<Change>> {
    let raw = run(&[
        // A rename is scored by the attributes of its files, and the working tree's are not part of either commit.
        &format!("--attr-source={base}"),
        "diff",
        "--no-relative",
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
        let kind = match status.as_bytes().first() {
            Some(b'A') => Kind::Added,
            Some(b'D') => Kind::Deleted,
            Some(b'M' | b'T') => Kind::Edited,
            Some(b'R') => Kind::Moved(path()?),
            _ => return Err(io::Error::other("unexpected git diff status")),
        };
        changes.push(Change {
            kind,
            path: path()?,
        });
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
