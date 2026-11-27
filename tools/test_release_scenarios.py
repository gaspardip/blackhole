import shutil
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

    def test_green_stdout_must_equal_the_case_advisory_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            plain, noted = root / "cases/plain", root / "cases/noted"
            for case in (plain, noted):
                for side in ("base", "head"):
                    (case / side).mkdir(parents=True)
                    (case / side / "file").write_text(side)
                (case / "expect").write_text("green\n")
            (noted / "advisory").write_text("a.ts:1:1 boundary/rule [advisory] note\n")
            cli = root / "blackhole"
            cli.touch()
            cli.chmod(0o755)
            for case, output, expected in (
                (noted, "a.ts:1:1 boundary/rule [advisory] note", 0),
                (noted, "", 1),
                (noted, "a.ts:2:1 boundary/rule [advisory] note", 1),
                (plain, "", 0),
                (plain, "a.ts:1:1 boundary/rule [advisory] note", 1),
            ):
                with self.subTest(case=case.name, output=output):
                    printed = f"echo '{output}'" if output else ":"
                    cli.write_text(f"#!/bin/sh\n{printed}\nexit 0\n")
                    only = root / "only"
                    shutil.rmtree(only, ignore_errors=True)
                    shutil.copytree(case, only / case.name)
                    done = subprocess.run(
                        [sys.executable, "-I", RUNNER, cli, only],
                        text=True, capture_output=True, check=False,
                    )
                    self.assertEqual(done.returncode, expected, done.stderr)
