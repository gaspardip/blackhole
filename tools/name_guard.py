"""Blocks names that must never appear in this repository.

The list lives outside the repository: the BLACKHOLE_BANNED environment variable
(CI secret) or the file named by BLACKHOLE_BANNED_FILE. Output never shows a match.

Exit status: 0 clean, 1 a name was found, 2 the guard could not run.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import traceback
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path
from collections.abc import Callable
from typing import Any

DEFAULT_LIST = Path.home() / ".config" / "blackhole-dev" / "banned-names"
MIN_LENGTH = 4
ZERO = re.compile(r"0+")
SHA = re.compile(r"[0-9a-f]{40}([0-9a-f]{24})?")
GITLINK = "160000"
TREE = "040000"
# `-t` lists directories as well, because an empty one has no file to carry its name.
RAW = ("--raw", "-z", "-t", "--no-renames", "--no-abbrev", "--ignore-submodules=none")
LIST = ("ls-tree", "-r", "-t", "-z")
# The `types` of the workflow. Any other action means the two files disagree.
ACTIONS = {
    "pull_request_target": {"opened", "edited", "reopened", "synchronize"},
    "issues": {"opened", "edited"},
    "issue_comment": {"created", "edited"},
    "release": {"created", "edited", "published"},
}
WRITERS = frozenset({"OWNER", "COLLABORATOR"})
STRANGERS = frozenset(
    {"CONTRIBUTOR", "FIRST_TIMER", "FIRST_TIME_CONTRIBUTOR", "MANNEQUIN", "MEMBER", "NONE"}
)
USAGE = "usage: name_guard.py staged | message <file> | push | range <base> <head> | tree | text | event"


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
        # git's own message would repeat the arguments, which can hold a rejected name.
        raise CannotRun(f"git {args[0]} failed with status {done.returncode}")
    return done.stdout


def decode(data: bytes) -> str:
    # Dropping NUL bytes makes UTF-16 text readable as its ASCII letters.
    return data.replace(b"\x00", b"").decode("utf-8", "replace")


def known(sha: str) -> bool:
    probe = ["git", "cat-file", "-e", f"{sha}^{{commit}}"]
    return subprocess.run(probe, capture_output=True, check=False).returncode == 0


def invalid_event() -> CannotRun:
    return CannotRun("the event payload is not what GitHub documents")


def string(value: object) -> str:
    if not isinstance(value, str):
        raise invalid_event()
    return value


def optional(value: object) -> str:
    return "" if value is None else string(value)


def object_id(value: object) -> str:
    sha = string(value)
    if not SHA.fullmatch(sha):
        raise invalid_event()
    return sha


def by_writer(item: dict[str, Any]) -> bool:
    association = string(item["author_association"])
    if association not in WRITERS | STRANGERS:
        raise invalid_event()
    return association in WRITERS


def load_event() -> tuple[str, dict[str, Any]]:
    name = os.environ.get("GITHUB_EVENT_NAME")
    path = os.environ.get("GITHUB_EVENT_PATH")
    if not name or not path:
        raise CannotRun("no event: GITHUB_EVENT_NAME and GITHUB_EVENT_PATH are not set")
    return name, json.loads(Path(path).read_text(encoding="utf-8"))


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
    """Parses LIST output: `mode type id TAB path NUL`."""
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
        if ZERO.fullmatch(entry.blob):  # a deletion adds nothing
            return
        self.name(f"{origin}file name", entry.path)
        if entry.mode in (GITLINK, TREE) or entry.blob in self.seen:
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
        for entry in tree_entries(git(*LIST, "HEAD")):
            self.entry(entry)

    def range(self, base: str, head: str) -> None:
        # An all-zero or unknown base (new ref, force push) means every commit is new.
        # A local tracking ref is no proof the destination has a commit, so nothing is excluded.
        if ZERO.fullmatch(base) or not known(base):
            self.commits(head)
        else:
            self.commits(head, base)

    def target(self, ref: str, target: str) -> str | None:
        """Scans non-commit objects and returns the commit the ref ends at, if any."""
        # A tag can point at another tag, and each one carries its own message.
        while (kind := git("cat-file", "-t", target).strip()) == b"tag":
            body = decode(git("cat-file", "tag", target))
            self.scan(f"tag {ref}", body)
            target = body.split("\n", 1)[0].removeprefix("object ")
        if kind == b"tree":
            for entry in tree_entries(git(*LIST, target)):
                self.entry(entry, f"{ref} ")
        elif kind == b"blob":
            self.entry(Entry("100644", target, ref))
        return target if kind == b"commit" else None

    def update_ref(
        self, ref: str, base: str, tip: str, *, local_ref: str | None = None
    ) -> None:
        self.name("ref", ref)
        if ZERO.fullmatch(tip):
            return
        if local_ref is not None:
            self.name("ref", local_ref)
        if (commit := self.target(ref, tip)) is not None:
            self.range(base, commit)

    def everything(self) -> None:
        listed = decode(git("for-each-ref", "--format=%(objectname) %(refname)"))
        for line in listed.splitlines():
            target, ref = line.split(" ", 1)
            self.name("ref", ref)
            self.target(ref, target)
        self.commits("--all")

    def written(
        self, item: dict[str, Any], kind: str, **fields: Callable[[object], str]
    ) -> bool:
        """Scans an item's text if its author can write here, and says whether they can."""
        # A writer's edit of a stranger's text is left alone too: whatever the scan compared,
        # its public result would depend on what the stranger had written.
        if not by_writer(item):
            return False
        for key, read in fields.items():
            self.scan(f"{kind} {key}", read(item[key]))
        return True

    def event(self, name: str, payload: dict[str, Any]) -> None:
        """Scans a GitHub event, and only what someone with write access wrote or pushed.

        A public pass or fail on a stranger's text would tell them whether a guessed name is banned.
        """
        # Organization roles do not give the same guarantees about who can write or edit.
        if string(payload["repository"]["owner"]["type"]) != "User":
            raise CannotRun("the trust rule only holds for a repository owned by a user")
        if name in ACTIONS and string(payload["action"]) not in ACTIONS[name]:
            raise invalid_event()
        match name:
            case "push":
                ref = string(payload["ref"])
                before, after = object_id(payload["before"]), object_id(payload["after"])
                self.update_ref(ref, before, after)
            case "pull_request_target":
                pull = payload["pull_request"]
                head, source = pull["head"], pull["head"]["repo"]
                # A head in another repository can be pushed to by people who cannot write here.
                # GitHub sends no repository at all once a fork is deleted.
                local = source is not None and string(source["full_name"]) == string(
                    payload["repository"]["full_name"]
                )
                if self.written(pull, "pull request", title=string, body=optional) and local:
                    tip = object_id(head["sha"])
                    if ZERO.fullmatch(tip):
                        raise invalid_event()
                    self.update_ref(string(head["ref"]), object_id(pull["base"]["sha"]), tip)
            case "issues":
                self.written(payload["issue"], "issue", title=string, body=optional)
            case "issue_comment":
                # The issue a comment sits on can be a stranger's, so only the comment is scanned.
                self.written(payload["comment"], "comment", body=string)
            case "release":
                release = payload["release"]
                self.scan("release name", optional(release["name"]))
                self.scan("release body", optional(release["body"]))
                self.name("ref", string(release["tag_name"]))
            case "schedule" | "workflow_dispatch":
                # GitHub sends no tag push events when more than three tags are pushed at once.
                self.everything()
            case _:
                raise CannotRun("no scan is defined for this event")

    def push(self, lines: str) -> None:
        for line in lines.splitlines():
            local_ref, local_sha, remote_ref, remote_sha = line.split()
            self.update_ref(remote_ref, remote_sha, local_sha, local_ref=local_ref)


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
            guard.env_text()
        case ["tree"]:
            guard.tree()
        case ["text"]:
            guard.env_text()
        case ["event"]:
            guard.event(*load_event())
        case _:
            raise CannotRun(USAGE)
    for hit in guard.hits:
        print(hit, file=sys.stderr)
    if guard.hits:
        print(f"name guard: {len(guard.hits)} banned name(s) found; remove them.", file=sys.stderr)
        return 1
    return 0


def main() -> int:
    # A local replacement ref would show the guard a different object than the one git sends.
    os.environ["GIT_NO_REPLACE_OBJECTS"] = "1"
    try:
        return run(sys.argv[1:])
    except CannotRun as error:
        print(f"name guard cannot run: {error}", file=sys.stderr)
    except Exception as error:
        # Status 1 means a name was found, and the error's own message can repeat its input.
        line = traceback.extract_tb(error.__traceback__)[-1].lineno
        print(f"name guard cannot run: {type(error).__name__} at line {line}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
