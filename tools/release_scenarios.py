"""Check the installed CLI against the repository's scenario expectations."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def run_case(cli: Path, case: Path, repo: Path, artifacts: Path) -> None:
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(
        GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
        GIT_AUTHOR_NAME="fixture", GIT_COMMITTER_NAME="fixture",
        GIT_AUTHOR_EMAIL="fixture@example.org", GIT_COMMITTER_EMAIL="fixture@example.org",
    )

    def git(*args: str) -> str:
        return subprocess.check_output(["git", *args], cwd=repo, env=env, text=True).strip()

    def commit(side: str) -> str:
        shutil.copytree(case / side, repo, dirs_exist_ok=True, ignore=shutil.ignore_patterns(".keep"))
        git("add", "-A")
        git("commit", "--allow-empty", "-qm", side)
        return git("rev-parse", "HEAD")

    repo.mkdir()
    git("init", "-q")
    base = commit("base")
    git("rm", "-rq", "--ignore-unmatch", ".")
    head = commit("head")

    verdict, *options = (case / "expect").read_text().splitlines()
    match options:
        case []:
            today = "2026-11-18"
        case [option] if option.startswith("today "):
            today = option.removeprefix("today ")
        case _:
            raise ValueError("invalid scenario options")
    cmd = [cli, "ci", "--base", base, "--head", head, "--today", today]
    report = case / "lcov.info"
    if report.exists():
        dir_ = artifacts / head / "reports"
        dir_.mkdir(parents=True)
        shutil.copyfile(report, dir_ / "lcov.info")
        cmd += ["--coverage-dir", artifacts]

    approvals = case / "approvals.json"
    if approvals.exists():
        data = json.loads(approvals.read_text())
        revisions = {"BASE": base, "HEAD": head}
        for review in data["reviews"]:
            if review.get("commit") in revisions:
                review["commit"] = revisions[review["commit"]]
        file = artifacts / "approvals.json"
        file.parent.mkdir(exist_ok=True)
        file.write_text(json.dumps(data))
        cmd += ["--approvals", file]

    result = subprocess.run(cmd, cwd=repo, env=env, capture_output=True, text=True, check=False)
    if verdict == "green":
        advisory = case / "advisory"
        expected = (0, advisory.read_text() if advisory.exists() else "")
        observed = (result.returncode, result.stdout)
    elif verdict.startswith("red "):
        expected = (1, "blackhole ci: red (" + ", ".join(verdict.split()[1:]) + ")")
        observed = (result.returncode, (result.stdout.splitlines() or [""])[-1])
    else:
        raise ValueError("invalid scenario verdict")
    if (*observed, result.stderr) != (*expected, ""):
        raise ValueError(f"expected {verdict}, got exit {result.returncode}\n{result.stdout}{result.stderr}")


def main(cli: Path, cases: Path) -> int:
    cases = sorted(cases.resolve().iterdir())
    if not cases:
        sys.exit("no scenario cases")
    failed = False
    for case in cases:
        try:
            with tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                run_case(cli.resolve(), case, root / "repo", root / "artifacts")
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            print(f"{case.name}: {error}", file=sys.stderr)
            failed = True
    if failed:
        return 1
    print(f"{len(cases)} scenario cases passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(*map(Path, sys.argv[1:])))
