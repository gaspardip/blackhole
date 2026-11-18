"""Blocks names that must never appear in this repository.

The list lives outside the repository: the BLACKHOLE_BANNED environment variable
(CI secret) or the file named by BLACKHOLE_BANNED_FILE. Output never shows a match.

Exit status: 0 clean, 1 a name was found, 2 the guard could not run.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_LIST = Path.home() / ".config" / "blackhole-dev" / "banned-names"
MIN_LENGTH = 4
ZERO = re.compile(r"^0+$")
GITLINK = "160000"
RAW = ("--raw", "-z", "--no-renames", "--no-abbrev")
USAGE = "usage: name_guard.py staged | message <file> | push | range <base> <head> | tree | text"


class CannotRun(Exception):
    pass


def normalize(text: str) -> str:
    return unicodedata.normalize("NFKC", text).casefold()


def load_patterns() -> list[re.Pattern[str]]:
    raw = os.environ.get("BLACKHOLE_BANNED")
    if raw is None:
        path = Path(os.environ.get("BLACKHOLE_BANNED_FILE", DEFAULT_LIST))
        if not path.is_file():
            raise CannotRun("no banned list: set BLACKHOLE_BANNED or create the list file")
        raw = path.read_text(encoding="utf-8")
    words = [
        "".join(char for char in normalize(entry) if char.isalnum())
        for entry in re.split(r"[\n,]", raw)
        if entry.strip()
    ]
    if not words:
        raise CannotRun("the banned list is empty")
    if any(len(word) < MIN_LENGTH for word in words):
        raise CannotRun(f"a banned list entry is shorter than {MIN_LENGTH} characters")
    # Up to three separators between letters covers any casing or spacing of the name.
    return [re.compile(r"[\W_]{0,3}".join(re.escape(char) for char in word)) for word in words]


def git(*args: str) -> bytes:
    done = subprocess.run(["git", *args], capture_output=True, check=False)
    if done.returncode != 0:
        raise CannotRun(f"git {args[0]} failed: {done.stderr.decode('utf-8', 'replace').strip()}")
    return done.stdout


def decode(data: bytes) -> str:
    # Dropping NUL bytes makes UTF-16 text readable as its ASCII letters.
    return data.replace(b"\x00", b"").decode("utf-8", "replace")


def known(sha: str) -> bool:
    probe = ["git", "cat-file", "-e", f"{sha}^{{commit}}"]
    return subprocess.run(probe, capture_output=True, check=False).returncode == 0


@dataclass(frozen=True)
class Entry:
    mode: str
    blob: str
    path: str


def diff_entries(raw: bytes) -> list[Entry]:
    """Parses RAW output: `:old new oldid newid status NUL path NUL`."""
    fields = raw.split(b"\x00")[:-1]
    entries = []
    for meta, path in zip(fields[0::2], fields[1::2], strict=True):
        _, mode, _, blob, _ = meta.decode().lstrip(":").split()
        entries.append(Entry(mode, blob, decode(path)))
    return entries


def tree_entries(raw: bytes) -> list[Entry]:
    """Parses `ls-tree -r -z` output: `mode type id TAB path NUL`."""
    entries = []
    for line in filter(None, raw.split(b"\x00")):
        meta, path = line.split(b"\t", 1)
        mode, _, blob = meta.decode().split()
        entries.append(Entry(mode, blob, decode(path)))
    return entries


@dataclass
class Guard:
    patterns: list[re.Pattern[str]]
    hits: list[str] = field(default_factory=list)
    seen: set[str] = field(default_factory=set)

    def redact(self, label: str) -> str:
        shown = normalize(label)
        for pattern in self.patterns:
            shown = pattern.sub("[banned]", shown)
        return shown if shown != normalize(label) else label

    def scan(self, label: str, text: str) -> None:
        body = normalize(text)
        for number, pattern in enumerate(self.patterns, start=1):
            for match in pattern.finditer(body):
                line = body.count("\n", 0, match.start()) + 1
                self.hits.append(f"{self.redact(label)}:{line} banned name #{number}")

    def name(self, kind: str, value: str) -> None:
        self.scan(f"{kind} {value}", value)

    def entry(self, entry: Entry, origin: str = "") -> None:
        if ZERO.match(entry.blob):  # a deletion adds nothing
            return
        self.name(f"{origin}file name", entry.path)
        if entry.mode == GITLINK or entry.blob in self.seen:
            return
        self.seen.add(entry.blob)
        # Blobs are read by id, so a path git prints differently than we decode it cannot hide one.
        # shortcut: one git process per blob; switch to `cat-file --batch` when the hook gets slow.
        self.scan(f"{origin}{entry.path}", decode(git("cat-file", "blob", entry.blob)))

    def env_text(self) -> None:
        for key in sorted(os.environ):
            if key.startswith("GUARD_TEXT_"):
                self.scan(key.removeprefix("GUARD_TEXT_"), os.environ[key])

    def commit(self, sha: str) -> None:
        short = sha[:9]
        self.scan(f"commit {short}", decode(git("cat-file", "commit", sha)))
        changed = git("diff-tree", "--root", "-r", "-m", "--no-commit-id", *RAW, sha)
        for entry in diff_entries(changed):
            self.entry(entry, f"{short} ")

    def commits(self, head: str, *exclude: str) -> None:
        listed = git("rev-list", head, "--not", *exclude).decode().split()
        for sha in listed:
            self.commit(sha)

    def staged(self) -> None:
        branch = subprocess.run(
            ["git", "symbolic-ref", "--short", "-q", "HEAD"], capture_output=True, check=False
        )
        self.name("branch", decode(branch.stdout).strip())
        for entry in diff_entries(git("diff", "--cached", *RAW)):
            self.entry(entry)

    def tree(self) -> None:
        for entry in tree_entries(git("ls-tree", "-r", "-z", "HEAD")):
            self.entry(entry)

    def range(self, base: str, head: str) -> None:
        # An all-zero or unknown base (new ref, force push) means every commit is new.
        if ZERO.match(base) or not known(base):
            self.commits(head)
        else:
            self.commits(head, base)

    def push(self, lines: str) -> None:
        for line in lines.splitlines():
            local_ref, local_sha, remote_ref, remote_sha = line.split()
            self.name("ref", remote_ref)
            if ZERO.match(local_sha):
                continue
            self.name("ref", local_ref)
            if git("cat-file", "-t", local_sha).strip() == b"tag":
                self.scan(f"tag {remote_ref}", decode(git("cat-file", "tag", local_sha)))
            if ZERO.match(remote_sha) or not known(remote_sha):
                self.commits(f"{local_sha}^{{commit}}", "--remotes")
            else:
                self.commits(f"{local_sha}^{{commit}}", remote_sha)


def run(argv: list[str]) -> int:
    guard = Guard(load_patterns())
    match argv:
        case ["staged"]:
            guard.staged()
        case ["message", path]:
            guard.scan("commit message", Path(path).read_text(encoding="utf-8", errors="replace"))
        case ["push"]:
            guard.push(sys.stdin.read())
        case ["range", base, head]:
            guard.range(base, head)
        case ["tree"]:
            guard.tree()
        case ["text"]:
            pass
        case _:
            raise CannotRun(USAGE)
    guard.env_text()
    for hit in guard.hits:
        print(hit, file=sys.stderr)
    if guard.hits:
        print(f"name guard: {len(guard.hits)} banned name(s) found; remove them.", file=sys.stderr)
        return 1
    return 0


def main() -> int:
    try:
        return run(sys.argv[1:])
    except CannotRun as error:
        print(f"name guard cannot run: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
