"""Skip release uploads only when their published bytes match."""

import base64
import hashlib
import json
import os
import subprocess
import sys
from itertools import count
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen


def digest(path: Path, algorithm: str) -> bytes:
    with path.open("rb") as artifact:
        return hashlib.file_digest(artifact, algorithm).digest()


def needs_upload(url: str, package: str, field: tuple[str, str], expected: str) -> bool:
    request = Request(url, headers={"Accept": "application/json", "User-Agent": "blackhole-release"})
    try:
        with urlopen(request, timeout=30) as response:
            metadata = json.load(response)
    except HTTPError as error:
        if error.code == 404:
            return True
        raise
    section, key = field
    if metadata[section][key] != expected:
        raise ValueError(f"{package} version already exists with a different digest")
    return False


def registry_uploads(tag: str, npm: Path, crate: Path) -> dict[str, bool]:
    version = tag.removeprefix("v")
    integrity = "sha512-" + base64.b64encode(digest(npm, "sha512")).decode("ascii")
    return {
        "npm": needs_upload(
            f"https://registry.npmjs.org/@gaspardip%2Fblackhole/{version}",
            "npm", ("dist", "integrity"), integrity,
        ),
        "crate": needs_upload(
            f"https://crates.io/api/v1/crates/blackhole-check/{version}",
            "crate", ("version", "checksum"), digest(crate, "sha256").hex(),
        ),
    }


def github_release(repo: str, tag: str) -> dict | None:
    url = f"{os.environ['GITHUB_API_URL']}/repos/{repo}/releases"
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
    }
    # Listing releases also finds drafts left by an interrupted upload.
    for page in count(1):
        request = Request(f"{url}?per_page=100&page={page}", headers=headers)
        with urlopen(request, timeout=30) as response:
            releases = json.load(response)
        if not isinstance(releases, list):
            raise ValueError("GitHub release lookup returned an invalid list")
        for release in releases:
            if release["tag_name"] == tag:
                return release
        if len(releases) < 100:
            return None


def publish_github(tag: str, npm: Path, crate: Path, sums: Path) -> None:
    repo = os.environ["GITHUB_REPOSITORY"]
    prerelease = os.environ["NPM_DIST_TAG"] == "next"
    checksums = {
        name: checksum
        for checksum, name in (line.split("  ", 1) for line in sums.read_text().splitlines())
    }
    expected = {
        npm.name: f"sha256:{checksums[npm.name]}",
        crate.name: f"sha256:{checksums[crate.name]}",
        sums.name: f"sha256:{digest(sums, 'sha256').hex()}",
    }
    release = github_release(repo, tag)
    draft = release is None or release["draft"]
    present = set()
    if release is not None:
        if release["prerelease"] is not prerelease:
            raise ValueError("GitHub release has a different prerelease setting")
        for asset in release["assets"]:
            name = asset["name"]
            if name not in expected or asset["digest"] != expected[name]:
                raise ValueError(f"GitHub release asset {name} has an unexpected digest")
            present.add(name)
    else:
        flags = ["--prerelease"] if prerelease else []
        subprocess.run([
            "gh", "release", "create", tag, "--repo", repo, "--verify-tag", "--draft",
            "--title", tag, "--notes", os.environ["RELEASE_NOTES"], *flags,
        ], check=True)
    missing = [str(path) for path in (npm, crate, sums) if path.name not in present]
    if missing:
        subprocess.run(["gh", "release", "upload", tag, "--repo", repo, *missing], check=True)
    if draft:
        subprocess.run(["gh", "release", "edit", tag, "--repo", repo, "--draft=false"], check=True)


if __name__ == "__main__":
    try:
        destination, npm_path, crate_path, sums_path = sys.argv[1:]
        ref = os.environ["GITHUB_REF_NAME"]
        if destination == "registries":
            for package, upload in registry_uploads(ref, Path(npm_path), Path(crate_path)).items():
                print(f"{package}={str(upload).lower()}")
        elif destination == "github":
            publish_github(ref, Path(npm_path), Path(crate_path), Path(sums_path))
        else:
            raise ValueError(f"unknown release destination {destination}")
    except HTTPError as error:
        sys.exit(f"release upload: lookup failed for {error.url} (HTTP {error.code})")
    except (KeyError, OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(f"release upload: {error}")
