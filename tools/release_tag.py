"""Validate a release tag before building any artifacts."""

import json
import os
import re
import subprocess
import sys
import tomllib
from collections.abc import Callable
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

NUMBER = r"(?:0|[1-9][0-9]*)"
TAG = re.compile(rf"v({NUMBER}\.{NUMBER}\.{NUMBER})(?:-rc\.({NUMBER}))?")
VERSION_FILES = {"Cargo.toml": "[package]", "Cargo.lock": "[[package]]"}


def verify_tag(repo: Path, tag: str, published_prerelease: Callable[[str], bool]) -> str:
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
        if ancestor(base, head) and only_version_changes(rc, base, head) and published_prerelease(rc):
            return "latest"
    raise ValueError("final tag needs a published GitHub prerelease ancestor with a version-only diff")


def github_prerelease(tag: str) -> bool:
    api = os.environ["GITHUB_API_URL"]
    repo = os.environ["GITHUB_REPOSITORY"]
    request = Request(f"{api}/repos/{repo}/releases/tags/{tag}", headers={
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
    })
    try:
        with urlopen(request, timeout=30) as response:
            release = json.load(response)
    except HTTPError as error:
        if error.code == 404:
            return False
        raise ValueError(f"GitHub release lookup failed (HTTP {error.code})") from None
    return release["tag_name"] == tag and release["prerelease"] is True and release["draft"] is False


if __name__ == "__main__":
    try:
        print(verify_tag(Path.cwd(), sys.argv[1], github_prerelease))
    except (ValueError, subprocess.CalledProcessError) as error:
        sys.exit(f"release: {error}")
