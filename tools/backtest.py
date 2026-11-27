"""Judge recently merged changes in a local clone with an installed blackhole."""
import argparse
import json
import os
import re
import subprocess
import tempfile
from collections import Counter
from dataclasses import asdict, dataclass, field
from datetime import UTC, datetime
from pathlib import Path

POLICY = ("blackhole.toml", ".blackhole/ledger")
OBJECTIONS = ".blackhole/objections"
STATES = {0: "green", 1: "red", 3: "waiting", 75: "no_verdict"}
FINDING = re.compile(r"^(.+?):([0-9]+):[0-9]+ ([a-z0-9-]+/[a-z0-9-]+)(?: |$)")
RED_GATES = re.compile(r"blackhole ci: red \(([a-z]+(?:, [a-z]+)*)\)")
GATE_MESSAGE = re.compile(
    r"^(?:protected: |objection: |coverage: |ledger line [0-9]+: )"
    r"|: ledger allows [0-9]+, found [0-9]+; run blackhole ledger$",
)


@dataclass(frozen=True)
class Change:
    commit: str
    base: str
    proposed: str | None = None


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    rule: str
    label: None = None


@dataclass
class Verdict:
    state: str
    today: str
    exit_code: int | None = None
    last_line: str | None = None
    findings: list[Finding] = field(default_factory=list)
    reason: str | None = None
    gates: list[str] = field(default_factory=list)
    raw_output: str | None = None


class CommandError(RuntimeError):
    def __init__(self, cwd: Path, result: subprocess.CompletedProcess[str]) -> None:
        self.result = result
        super().__init__(
            f"{result.args[0]} in {cwd} exited {result.returncode}: {result.stdout.strip()}",
        )


def environment(home: Path) -> dict[str, str]:
    env = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
    env.update(
        HOME=str(home),
        XDG_CONFIG_HOME=str(home),
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_NOSYSTEM="1",
        GIT_AUTHOR_NAME="Blackhole Backtest",
        GIT_AUTHOR_EMAIL="backtest@example.invalid",
        GIT_AUTHOR_DATE="2000-01-01T00:00:00Z",
        GIT_COMMITTER_NAME="Blackhole Backtest",
        GIT_COMMITTER_EMAIL="backtest@example.invalid",
        GIT_COMMITTER_DATE="2000-01-01T00:00:00Z",
    )
    return env


def run(
    cwd: Path, env: dict[str, str], *args: str | Path, check: bool = True,
) -> subprocess.CompletedProcess[str]:
    process = subprocess.run(
        args, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    result = subprocess.CompletedProcess(
        args, process.returncode, process.stdout.decode("utf-8", errors="surrogateescape"),
    )
    if check and result.returncode:
        raise CommandError(cwd, result)
    return result


def git(
    cwd: Path, env: dict[str, str], *args: str | Path, check: bool = True,
) -> subprocess.CompletedProcess[str]:
    return run(
        cwd, env, "git", "--no-replace-objects", "--no-lazy-fetch", "--literal-pathspecs",
        "-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false",
        "-c", "commit.gpgSign=false", "-c", "rerere.enabled=false",
        "-c", "maintenance.auto=false", *args, check=check,
    )


def history(repo: Path, env: dict[str, str], mode: str, limit: int) -> list[Change]:
    parents = "2" if mode == "merge" else "1"
    rows = git(
        repo, env, "rev-list", "--first-parent", "--parents",
        f"--min-parents={parents}", f"--max-parents={parents}", f"--max-count={limit}",
        "refs/remotes/origin/HEAD", "--",
    ).stdout.splitlines()
    return [Change(*row.split()) for row in rows]


def dated_ci(
    tree: Path, env: dict[str, str], binary: Path, base: str, head: str, today: str,
) -> subprocess.CompletedProcess[str]:
    return run(tree, env, binary, "ci", "--base", base, "--head", head, "--today", today, check=False)


def baseline(tree: Path, env: dict[str, str], binary: Path, base: str, today: str) -> str:
    paths = git(tree, env, "ls-files", "-z", "--", OBJECTIONS).stdout.split("\0")
    has_objections = OBJECTIONS in paths
    if has_objections:
        # Init uses the wall clock, so validate objections with dated CI before setting them aside.
        validation = dated_ci(tree, env, binary, base, base, today)
        if validation.returncode not in (0, 1):
            raise CommandError(tree, validation)
        (tree / OBJECTIONS).unlink()
    output = run(tree, env, binary, "init").stdout
    if has_objections:
        git(tree, env, "restore", "--", OBJECTIONS)
    for line in output.splitlines():
        action, _, path = line.partition(" ")
        if action not in {"Wrote", "Created"} or path in POLICY:
            continue
        target = tree / path
        if not path or not target.resolve().is_relative_to(tree) or target == tree:
            raise RuntimeError(f"init reported an invalid path: {path!r}")
        if git(tree, env, "ls-files", "--", path).stdout:
            git(tree, env, "restore", "--", path)
        else:
            target.unlink()
    git(tree, env, "add", "-f", "--", *POLICY)
    git(tree, env, "commit", "--allow-empty", "-m", "Initialize backtest policy")
    return git(tree, env, "rev-parse", "HEAD").stdout.strip()


def judge(tree: Path, env: dict[str, str], binary: Path, change: Change) -> Verdict:
    timestamp = git(tree, env, "show", "-s", "--format=%ct", change.commit).stdout.strip()
    today = datetime.fromtimestamp(int(timestamp), UTC).date().isoformat()
    try:
        base = baseline(tree, env, binary, change.base, today)
    except CommandError as error:
        return Verdict(
            "baseline_failure", today, error.result.returncode, raw_output=error.result.stdout,
        )
    except (RuntimeError, OSError) as error:
        return Verdict("baseline_failure", today, reason=str(error))
    mainline = ("-m", "1") if change.proposed else ()
    replay = git(tree, env, "cherry-pick", "--no-commit", *mainline, change.commit, check=False)
    if replay.returncode:
        conflicts = git(tree, env, "diff", "--name-only", "--diff-filter=U", "-z").stdout
        if not conflicts:
            raise RuntimeError(f"replaying {change.commit} failed: {replay.stdout.strip()}")
        paths = ", ".join(conflicts.rstrip("\0").split("\0"))
        return Verdict("skipped", today, reason=f"cherry-pick conflict: {paths}")

    git(tree, env, "commit", "--allow-empty", "-m", f"Replay {change.commit}")
    head = git(tree, env, "rev-parse", "HEAD").stdout.strip()
    result = dated_ci(tree, env, binary, base, head, today)
    lines = result.stdout.splitlines()
    state = STATES.get(result.returncode, "error")
    last_line = lines[-1] if lines else None
    gates = []
    if state == "red" and last_line and (match := RED_GATES.fullmatch(last_line)):
        gates = match[1].split(", ")
    findings = []
    if state in {"green", "red", "waiting"}:
        for line in lines:
            if GATE_MESSAGE.search(line):
                continue
            if match := FINDING.match(line):
                path, number, rule = match.groups()
                findings.append(Finding(path, int(number), rule))
    return Verdict(state, today, result.returncode, last_line, findings, gates=gates)


def backtest(repo: Path, binary: Path, out: Path, mode: str, limit: int) -> None:
    source_env = environment(Path(os.devnull))
    source_paths = [
        Path(git(repo, source_env, "rev-parse", "--path-format=absolute", option)
             .stdout.removesuffix("\n")).resolve()
        for option in ("--show-toplevel", "--git-dir", "--git-common-dir")
    ]
    if repo != source_paths[0]:
        raise RuntimeError(f"source repo must be its worktree root: {source_paths[0]}")
    out = out.resolve()
    report = (out / repo.name).resolve()
    for directory in (out, report):
        if any(directory.is_relative_to(source) for source in source_paths):
            raise RuntimeError(f"output or report directory is inside the source clone: {directory}")
    out.mkdir(parents=True, exist_ok=True)
    states = Counter(dict.fromkeys((*STATES.values(), "error", "skipped", "baseline_failure"), 0))
    rules: Counter[str] = Counter()
    with tempfile.TemporaryDirectory(dir=out) as scratch:
        temp = Path(scratch)
        home = temp / "home"
        home.mkdir()
        env = environment(home)
        changes = history(repo, env, mode, limit)
        object_format, objects = git(
            repo, env, "rev-parse", "--show-object-format", "--path-format=absolute",
            "--git-path", "objects",
        ).stdout.removesuffix("\n").split("\n", 1)
        if "\n" in objects:
            raise RuntimeError("source object directory contains a newline")
        results = temp / "results.jsonl"
        with results.open("w", encoding="utf-8") as rows:
            for change in changes:
                with tempfile.TemporaryDirectory(dir=temp) as replay:
                    tree = Path(replay)
                    git(tree, env, "init", "--template=", f"--object-format={object_format}")
                    (tree / ".git/objects/info/alternates").write_bytes(os.fsencode(objects) + b"\n")
                    git(tree, env, "switch", "--detach", change.base)
                    verdict = judge(tree, env, binary, change)
                rows.write(json.dumps({**asdict(change), **asdict(verdict)}) + "\n")
                states[verdict.state] += 1
                if verdict.state in {"green", "red"}:
                    rules.update({finding.rule for finding in verdict.findings})
        judged = states["green"] + states["red"]
        summary = {
            "judged": judged,
            "skipped": states["skipped"],
            "states": states,
            "rules": {
                rule: {"changes": count, "flag_rate": count / judged}
                for rule, count in sorted(rules.items())
            },
        }
        summary_file = temp / "summary.json"
        summary_file.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        report.mkdir(exist_ok=True)
        results.replace(report / results.name)
        summary_file.replace(report / summary_file.name)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--limit", required=True, type=int)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--mode", choices=("merge", "squash"), default="merge")
    args = parser.parse_args()
    if args.limit < 1:
        parser.error("--limit must be positive")
    try:
        backtest(args.repo.resolve(), args.binary.resolve(), args.out, args.mode, args.limit)
    except (OSError, RuntimeError) as error:
        parser.exit(1, f"backtest: {error}\n")


if __name__ == "__main__":
    main()
