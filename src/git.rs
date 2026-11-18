use crate::verdict::Change;
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Read, Write},
    process::{Child, ChildStdout, Command, Output, Stdio},
};

fn cmd() -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("--no-replace-objects");
    cmd
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
fn run(args: &[&str]) -> io::Result<Vec<u8>> {
    let out = cmd().args(args).output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(failure(&out))
    }
}
pub(crate) fn commit(rev: &str) -> io::Result<String> {
    let bytes = run(&[
        "rev-parse",
        "--verify",
        "--end-of-options",
        &format!("{rev}^{{commit}}"),
    ])?;
    Ok(std::str::from_utf8(&bytes)
        .map_err(io::Error::other)?
        .trim_end()
        .into())
}
pub(crate) fn ancestor(base: &str, head: &str) -> io::Result<bool> {
    let out = cmd()
        .args(["merge-base", "--is-ancestor", base, head])
        .output()?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(failure(&out)),
    }
}

pub(crate) struct Tree(BTreeMap<String, Option<String>>);
impl Tree {
    pub(crate) fn load(sha: &str) -> io::Result<Self> {
        let mut entries = BTreeMap::new();
        let bytes = run(&["ls-tree", "--full-tree", "-r", "-t", "-z", sha])?;
        for line in std::str::from_utf8(&bytes)
            .map_err(io::Error::other)?
            .split_terminator('\0')
        {
            let (header, path) = line
                .split_once('\t')
                .ok_or_else(|| io::Error::other("invalid git tree"))?;
            let parts: Vec<_> = header.split(' ').collect();
            let &[mode, kind, id] = parts.as_slice() else {
                return Err(io::Error::other("invalid tree entry"));
            };
            let blob =
                (kind == "blob" && matches!(mode, "100644" | "100755")).then(|| id.to_owned());
            entries.insert(path.into(), blob);
        }
        Ok(Self(entries))
    }
    pub(crate) fn judge_file(&self, path: &str, batch: &mut Batch) -> io::Result<Option<Vec<u8>>> {
        match self.0.get(path) {
            None => Ok(None),
            Some(Some(id)) => batch.read(id).map(Some),
            Some(None) => Err(io::Error::other(format!("{path} must be a regular blob"))),
        }
    }
    pub(crate) fn blobs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .filter_map(|(path, id)| Some((path.as_str(), id.as_deref()?)))
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
    let mut fields = std::str::from_utf8(&raw)
        .map_err(io::Error::other)?
        .split_terminator('\0');
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
        let mut child = cmd()
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
        drop(self.child.stdin.take());
        if self.child.wait()?.success() {
            Ok(())
        } else {
            Err(io::Error::other("git cat-file failed"))
        }
    }
}
impl Drop for Batch {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
