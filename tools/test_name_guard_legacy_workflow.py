from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

from test_name_guard import GUARD, WORDS, Repo, git, isolated_env

# Keep the original scan step executable after the workflow switches commands.
LEGACY_SCAN = r"""set -eu
git init -q repo
cd repo
git remote add origin "${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}.git"
git fetch -q origin "+refs/heads/*:refs/remotes/origin/*" "+refs/tags/*:refs/tags/*"
# The guard always comes from the default branch; the scanned refs are data.
guard="${RUNNER_TEMP}/name_guard.py"
git show "origin/${DEFAULT_BRANCH}:tools/name_guard.py" > "${guard}"
case "${GITHUB_EVENT_NAME}" in
  push)
    if [ "${GITHUB_REF_TYPE}" = tag ]; then
      GUARD_TEXT_TAG_OBJECT="$(git cat-file -p "${GITHUB_REF}")"
      export GUARD_TEXT_TAG_OBJECT
    fi
    python3 -I "${guard}" range "${PUSH_BEFORE}" "${GITHUB_SHA}"
    ;;
  pull_request_target)
    git fetch -q origin "+refs/pull/${PR_NUMBER}/head:refs/remotes/pr/head"
    python3 -I "${guard}" range "${PR_BASE_SHA}" "${PR_HEAD_SHA}"
    ;;
  *)
    python3 -I "${guard}" text
    ;;
esac
"""


class LegacyWorkflow(Repo):
    def test_legacy_workflow_scans_ranges_and_environment_text(self) -> None:
        self.write("tools/name_guard.py", GUARD.read_text(encoding="utf-8"))
        base = self.commit("chore: install guard")
        clean = self.commit("chore: clean update")
        self.write("example.txt", "Quokka\n")
        dirty = self.commit("chore: add fixture")
        origin = self.dir / "origin.git"
        git(self.dir, "clone", "--bare", "-q", ".", str(origin))

        events = {
            "push": "GUARD_TEXT_REF",
            "pull_request_target": "GUARD_TEXT_PR_TITLE",
            "issue_comment": "GUARD_TEXT_COMMENT",
        }
        for event, key in events.items():
            cases = {
                "clean": (clean, "", ""),
                "text": (clean, "Quokka", f"{key.removeprefix('GUARD_TEXT_')}:1"),
            }
            if event != "issue_comment":
                cases["commit"] = (dirty, "", "example.txt:1")
            for name, (head, value, label) in cases.items():
                with self.subTest(event=event, case=name), tempfile.TemporaryDirectory() as tmp:
                    directory = Path(tmp)
                    git(origin, "update-ref", "refs/pull/7/head", head)
                    env = isolated_env(directory) | {
                        "BLACKHOLE_BANNED": WORDS,
                        "GITHUB_SERVER_URL": self.dir.as_uri(),
                        "GITHUB_REPOSITORY": "origin",
                        "DEFAULT_BRANCH": "main",
                        "GITHUB_EVENT_NAME": event,
                        "GITHUB_REF_TYPE": "branch",
                        "GITHUB_SHA": head,
                        "PUSH_BEFORE": base,
                        "PR_NUMBER": "7",
                        "PR_BASE_SHA": base,
                        "PR_HEAD_SHA": head,
                        "RUNNER_TEMP": str(directory),
                        key: value,
                    }
                    done = subprocess.run(
                        ["sh", "-c", LEGACY_SCAN], cwd=directory, env=env,
                        capture_output=True, text=True, check=False,
                    )
                    self.assertEqual(done.returncode, int(bool(label)), done.stderr)
                    if label:
                        self.assertIn(f"{label} banned name #1", done.stderr)
