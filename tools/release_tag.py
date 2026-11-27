"""Validate a release tag before building any artifacts."""

import json
import os
import re
import subprocess
import sys
import tomllib
from collections.abc import Callable
from itertools import count
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

NUMBER = r"(?:0|[1-9][0-9]*)"
TAG = re.compile(rf"v({NUMBER}\.{NUMBER}\.{NUMBER})(?:-rc\.({NUMBER}))?")
VERSION_FILES = {"Cargo.toml": "[package]", "Cargo.lock": "[[package]]"}


def verify_tag(repo: Path, tag: str, successful_release: Callable[[str, str], bool]) -> str:
    def git_bytes(*args: str) -> bytes:
        return subprocess.check_output(["git", *args], cwd=repo)

    def git(*args: str) -> str:
        return git_bytes(*args).decode()

    def ancestor(base: str, head: str) -> bool:
        status = subprocess.run(
            ["git", "merge-base", "--is-ancestor", base, head], cwd=repo, check=False
        ).returncode
        if status not in (0, 1):
            raise ValueError("git could not check commit ancestry")
        return status == 0

    def only_version_changes(rc: str, base: str, head: str) -> bool:
        changes = git("diff", "--no-renames", "--name-only", base, head, "--").splitlines()
        if sorted(changes) != sorted(VERSION_FILES):
            return False
        if git("diff", "--no-renames", "--summary", base, head, "--"):
            return False
        for path, section in VERSION_FILES.items():
            old = git_bytes("show", f"{base}:{path}")
            new = git_bytes("show", f"{head}:{path}")
            prefix = f'{section}\nname = "blackhole-check"\nversion = "'
            before = f'{prefix}{rc[1:]}"\n'.encode()
            after = f'{prefix}{tag[1:]}"\n'.encode()
            if old.count(before) != 1 or old.replace(before, after, 1) != new:
                return False
        return True

    match = TAG.fullmatch(tag)
    if match is None:
        raise ValueError("expected a vX.Y.Z or vX.Y.Z-rc.N release tag")
    head = git("rev-parse", "HEAD").strip()
    if git("rev-parse", f"refs/tags/{tag}^{{commit}}").strip() != head:
        raise ValueError("release tag does not resolve to the checked-out commit")
    if not ancestor(head, "refs/remotes/origin/main"):
        raise ValueError("release commit is not reachable from origin/main")
    manifest = tomllib.loads(git("show", f"{head}:Cargo.toml"))
    if manifest["package"]["version"] != tag[1:]:
        raise ValueError("release tag does not match the Cargo version")
    if match.group(2) is not None:
        return "next"

    for rc in git("tag", "--list", f"{tag}-rc.*").splitlines():
        candidate = TAG.fullmatch(rc)
        if candidate is None or candidate.group(1) != match.group(1):
            continue
        base = git("rev-parse", f"refs/tags/{rc}^{{commit}}").strip()
        if ancestor(base, head) and only_version_changes(rc, base, head) and successful_release(rc, base):
            return "latest"
    raise ValueError("final tag needs a successfully released candidate ancestor with a version-only diff")


def github_candidate_succeeded(tag: str, sha: str) -> bool:
    api = os.environ["GITHUB_API_URL"]
    repo = os.environ["GITHUB_REPOSITORY"]
    url = f"{api}/repos/{repo}/actions/workflows/release.yml/runs"
    for page in count(1):
        request = Request(f"{url}?event=push&head_sha={sha}&status=success&per_page=100&page={page}", headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
        })
        try:
            with urlopen(request, timeout=30) as response:
                runs = json.load(response)["workflow_runs"]
        except HTTPError as error:
            raise ValueError(f"GitHub workflow run lookup failed (HTTP {error.code})") from error
        if any(run["head_branch"] == tag and run["head_sha"] == sha
               and run["event"] == "push" and run["conclusion"] == "success"
               and run["run_attempt"] == 1 for run in runs):
            return True
        if len(runs) < 100:
            return False


if __name__ == "__main__":
    try:
        print(verify_tag(Path.cwd(), sys.argv[1], github_candidate_succeeded))
    except (ValueError, subprocess.CalledProcessError) as error:
        sys.exit(f"release: {error}")
