from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from shutil import copy

from test_name_guard import GUARD, WORDS, Repo, git, isolated_env, run

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


def fake_gh(directory: Path, script: str) -> dict[str, str]:
    gh = directory / "gh"
    gh.write_text(script, encoding="utf-8")
    gh.chmod(0o755)
    env = isolated_env(directory)
    env["PATH"] = f"{directory}{os.pathsep}{env['PATH']}"
    return env


class LandedMerge(Repo):
    def squash(self, origin: Path, landed: str) -> subprocess.CompletedProcess[str]:
        script = self.dir / "bin" / SCRIPT.name
        guard = self.dir / "tools" / GUARD.name
        script.parent.mkdir()
        guard.parent.mkdir()
        copy(SCRIPT, script)
        copy(GUARD, guard)
        env = fake_gh(self.dir, GH_SQUASH) | {
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

    def test_a_hit_in_the_combined_squash_blocks_the_merge(self) -> None:
        self.write("example.txt", "first\n\nlast\n")
        ancestor = self.commit("chore: start")
        git(self.dir, "checkout", "-q", "-b", "pull")
        self.write("example.txt", "first\n\nkka\n")
        head = self.commit("chore: change last line")
        git(self.dir, "checkout", "-q", "main")
        self.write("example.txt", "quo\n\nlast\n")
        base = self.commit("chore: change first line")
        for tip in (base, head):
            clean = run(["range", ancestor, tip], self.dir)
            self.assertEqual(clean.returncode, 0, clean.stderr)

        tree = self.loose_tree("example.txt", "quo\n\nkka\n")
        landed = git(self.dir, "commit-tree", tree, "-p", base, "-m", "chore: review (#7)")
        git(self.dir, "update-ref", "refs/heads/fixture", landed)
        origin = self.dir / "origin"
        git(self.dir, "clone", "--bare", "-q", ".", str(origin))
        git(origin, "update-ref", "refs/pull/7/head", head)
        git(self.dir, "remote", "add", "origin", str(origin))

        done = self.squash(origin, landed)

        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn("example.txt:1 banned name #1", done.stderr)
        self.assertEqual(git(origin, "rev-parse", "refs/heads/main"), base)


class MergeOutside(unittest.TestCase):
    def test_a_pull_request_into_another_branch_is_refused_before_any_merge(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            calls = directory / "calls"
            env = fake_gh(directory, GH) | {
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
