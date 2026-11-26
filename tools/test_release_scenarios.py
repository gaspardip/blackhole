import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RUNNER = Path(__file__).with_name("release_scenarios.py")


class ReleaseScenarios(unittest.TestCase):
    def test_installed_cli_must_match_exit_code_stdout_and_stderr(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = root / "cases/example"
            for side in ("base", "head"):
                (case / side).mkdir(parents=True)
                (case / side / "file").write_text(side)
            (case / "expect").write_text("red behind coverage\ntoday 2026-11-20\n")
            cli = root / "blackhole"
            cli.touch()
            cli.chmod(0o755)
            correct = "echo 'blackhole ci: red (behind, coverage)'"
            for script, expected in (
                (f"{correct}\nexit 1", 0),
                (f"{correct}\nexit 0", 1),
                ("echo 'blackhole ci: red (coverage)'\nexit 1", 1),
                (f"{correct}\necho unexpected >&2\nexit 1", 1),
            ):
                with self.subTest(script=script):
                    cli.write_text(
                        "#!/bin/sh\n"
                        'test "$1 $2 $4 $6 $7" = "ci --base --head --today 2026-11-20" || exit 2\n'
                        'test "$(git show "$3:file")" = base || exit 2\n'
                        'test "$(git show "$5:file")" = head || exit 2\n'
                        f"{script}\n"
                    )
                    done = subprocess.run(
                        [sys.executable, "-I", RUNNER, cli, case.parent],
                        text=True, capture_output=True, check=False,
                    )
                    self.assertEqual(done.returncode, expected, done.stderr)
                    if expected:
                        self.assertIn("example: expected red behind coverage", done.stderr)
