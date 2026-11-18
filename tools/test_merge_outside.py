from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent.with_name("bin") / "merge-outside"
# Stands in for the GitHub CLI: records each call and names the branch a pull request targets.
GH = '#!/bin/sh\necho "$*" >> "${GH_CALLS}"\necho "${GH_BASE}"\n'


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
