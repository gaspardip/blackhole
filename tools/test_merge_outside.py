from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from shutil import copy

from test_name_guard import GUARD, WORDS, Repo, git

SCRIPT = Path(__file__).parent.with_name("bin") / "merge-outside"
# Stands in for the GitHub CLI: records each call and names the branch a pull request targets.
GH = '#!/bin/sh\necho "$*" >> "${GH_CALLS}"\necho "${GH_BASE}"\n'
GH_SQUASH = """#!/bin/sh
set -eu
case "$*" in
  'pr view 7 --json baseRefName --jq .baseRefName') echo main ;;
  'pr view 7 --json title --jq .title') echo 'chore: review' ;;
  'pr view 7 --json body --jq .body') ;;
  'pr merge 7 --squash --match-head-commit '*)
    git -C "${GH_ORIGIN}" update-ref refs/heads/main "${GH_LANDED}"
    ;;
  *) exit 2 ;;
esac
"""


class LandedMerge(Repo):
    def squash(self, origin: Path, landed: str) -> subprocess.CompletedProcess[str]:
        script = self.dir / "bin" / SCRIPT.name
        guard = self.dir / "tools" / GUARD.name
        script.parent.mkdir()
        guard.parent.mkdir()
        copy(SCRIPT, script)
        copy(GUARD, guard)
        gh = self.dir / "gh"
        gh.write_text(GH_SQUASH, encoding="utf-8")
        gh.chmod(0o755)
        env = {
            "PATH": f"{self.dir}{os.pathsep}{os.environ['PATH']}",
            "HOME": str(self.dir),
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_SYSTEM": os.devnull,
            "BLACKHOLE_BANNED": WORDS,
            "GH_ORIGIN": str(origin),
            "GH_LANDED": landed,
        }
        return subprocess.run(
            [str(script), "7"], env=env, capture_output=True, text=True, check=False
        )

    def test_a_main_tag_cannot_hide_the_landed_commit(self) -> None:
        base = self.commit("chore: start")
        origin = self.dir / "origin"
        git(self.dir, "clone", "--bare", "-q", ".", str(origin))
        head = git(origin, "commit-tree", f"{base}^{{tree}}", "-p", base, "-m", "chore: review")
        git(origin, "update-ref", "refs/pull/7/head", head)
        git(self.dir, "remote", "add", "origin", str(origin))
        git(self.dir, "fetch", "-q", "origin", "refs/heads/main")
        git(origin, "tag", "main", base)
        landed = git(
            origin, "commit-tree", f"{base}^{{tree}}", "-p", base,
            "-m", "chore: review (#7)", "-m", "Co-authored-by: Quokka <q@example.invalid>",
        )

        done = self.squash(origin, landed)

        self.assertEqual(git(origin, "rev-parse", "refs/heads/main"), landed)
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn(f"commit {landed[:9]}:", done.stderr)
        self.assertIn("banned name #1", done.stderr)


class MergeOutside(unittest.TestCase):
    def test_a_pull_request_into_another_branch_is_refused_before_any_merge(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            calls = Path(tmp) / "calls"
            gh = Path(tmp) / "gh"
            gh.write_text(GH, encoding="utf-8")
            gh.chmod(0o755)
            env = {
                "PATH": f"{tmp}{os.pathsep}{os.environ['PATH']}",
                "GH_CALLS": str(calls),
                "GH_BASE": "release",
            }
            done = subprocess.run(
                [str(SCRIPT), "7"], env=env, capture_output=True, text=True, check=False
            )
            self.assertEqual(done.returncode, 1, done.stderr)
            self.assertIn("does not target main", done.stderr)
            self.assertEqual(calls.read_text(encoding="utf-8").count("\n"), 1)


if __name__ == "__main__":
    unittest.main()
