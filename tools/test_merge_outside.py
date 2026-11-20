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
GH_SQUASH = r"""#!/bin/sh
set -eu
target="${GH_TARGET:-main}"
case "$*" in
  'pr view 7 --json baseRefName --jq .baseRefName')
    if [ -f "${HOME}/base-seen" ]; then
      if [ -n "${GH_ADVANCED_BASE:-}" ]; then
        git -C "${GH_ORIGIN}" update-ref refs/heads/main "${GH_ADVANCED_BASE}"
      fi
      echo "${target}"
    else
      touch "${HOME}/base-seen"
      echo main
    fi
    ;;
  'pr view 7 --json title --jq .title') echo 'chore: review' ;;
  'pr view 7 --json body --jq .body') ;;
  'pr merge 7 --squash --match-head-commit '*)
    touch "${HOME}/merge-called"
    if [ "${GH_ENQUEUE:-0}" = 1 ]; then
      echo "queued for ${target}"
    else
      git -C "${GH_ORIGIN}" update-ref "refs/heads/${target}" "${GH_LANDED}"
      printf 'MERGED %s %s\n' "${target}" "${GH_LANDED}" > "${HOME}/merge-state"
    fi
    ;;
  'pr view 7 --json state,baseRefName,mergeCommit --jq '*)
    if [ -f "${HOME}/merge-state" ]; then
      cat "${HOME}/merge-state"
    else
      printf 'OPEN %s\n' "${target}"
    fi
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
    def squash(self, origin: Path, landed: str, **scenario: str) -> subprocess.CompletedProcess[str]:
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
        } | scenario
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

    def split_squash_inputs(self) -> tuple[str, str, str]:
        self.write("example.txt", "first\n\nlast\n")
        ancestor = self.commit("chore: start")
        git(self.dir, "checkout", "-q", "-b", "pull")
        self.write("example.txt", "first\n\nkka\n")
        head = self.commit("chore: change last line")
        git(self.dir, "checkout", "-q", "main")
        self.write("example.txt", "quo\n\nlast\n")
        base = self.commit("chore: change first line")
        return ancestor, base, head

    def prepare_split_squash(self, base: str, head: str) -> tuple[Path, str]:
        tree = self.loose_tree("example.txt", "quo\n\nkka\n")
        landed = git(self.dir, "commit-tree", tree, "-p", base, "-m", "chore: review (#7)")
        git(self.dir, "update-ref", "refs/heads/fixture", landed)
        origin = self.dir / "origin"
        git(self.dir, "clone", "--bare", "-q", ".", str(origin))
        git(origin, "update-ref", "refs/pull/7/head", head)
        git(self.dir, "remote", "add", "origin", str(origin))
        return origin, landed

    def test_a_hit_in_the_combined_squash_blocks_the_merge(self) -> None:
        ancestor, base, head = self.split_squash_inputs()
        for tip in (base, head):
            clean = run(["range", ancestor, tip], self.dir)
            self.assertEqual(clean.returncode, 0, clean.stderr)

        origin, landed = self.prepare_split_squash(base, head)
        done = self.squash(origin, landed)

        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn("example.txt:1 banned name #1", done.stderr)
        self.assertEqual(git(origin, "rev-parse", "refs/heads/main"), base)

    def test_a_base_replacement_cannot_hide_a_squash_hit(self) -> None:
        ancestor, base, head = self.split_squash_inputs()
        origin, landed = self.prepare_split_squash(base, head)
        replacement = git(
            self.dir, "commit-tree", f"{ancestor}^{{tree}}", "-p", ancestor, "-m", "chore: replace base",
        )
        git(self.dir, "replace", base, replacement)

        done = self.squash(origin, landed)

        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn("example.txt:1 banned name #1", done.stderr)
        self.assertFalse((self.dir / "merge-called").exists())

    def clean_pull(self) -> tuple[Path, str, str]:
        base = self.commit("chore: start")
        origin = self.dir / "origin"
        git(self.dir, "clone", "--bare", "-q", ".", str(origin))
        head = git(origin, "commit-tree", f"{base}^{{tree}}", "-p", base, "-m", "chore: review")
        git(origin, "update-ref", "refs/pull/7/head", head)
        landed = git(origin, "commit-tree", f"{base}^{{tree}}", "-p", base, "-m", "chore: review (#7)")
        git(self.dir, "remote", "add", "origin", str(origin))
        return origin, base, landed

    def test_a_retargeted_pull_request_is_refused_before_merge(self) -> None:
        origin, base, landed = self.clean_pull()
        git(origin, "update-ref", "refs/heads/release", base)

        done = self.squash(origin, landed, GH_TARGET="release")

        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertEqual(done.stderr, "merge-outside: pull request 7 does not target main\n")
        self.assertEqual(git(origin, "rev-parse", "refs/heads/release"), base)

    def test_an_advanced_base_is_refused_before_merge(self) -> None:
        origin, base, _ = self.clean_pull()
        tree = f"{base}^{{tree}}"
        advanced = git(origin, "commit-tree", tree, "-p", base, "-m", "chore: advance main")
        landed = git(origin, "commit-tree", tree, "-p", advanced, "-m", "chore: review (#7)")

        done = self.squash(origin, landed, GH_ADVANCED_BASE=advanced)

        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertEqual(done.stderr, "merge-outside: could not confirm preview base for pull request 7\n")
        self.assertFalse((self.dir / "merge-called").exists())
        self.assertEqual(git(origin, "rev-parse", "refs/heads/main"), advanced)

    def test_an_enqueued_pull_request_is_not_reported_as_merged(self) -> None:
        origin, base, landed = self.clean_pull()

        done = self.squash(origin, landed, GH_ENQUEUE="1")

        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertEqual(done.stderr, "merge-outside: could not confirm merge of pull request 7\n")
        self.assertEqual(done.stdout, "")
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
