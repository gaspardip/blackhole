import json
import os
import shlex
import subprocess
import sys
import tempfile
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class Backtest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        # BACKTEST_BINARY can point to an already built blackhole from this checkout.
        override = os.environ.get("BACKTEST_BINARY")
        if override:
            cls.binary = Path(override).resolve()
            return
        build = subprocess.check_output(
            ["cargo", "build", "--locked", "--bin", "blackhole", "--message-format=json"],
            cwd=ROOT, text=True,
        )
        cls.binary = next(
            Path(event["executable"])
            for line in reversed(build.splitlines())
            if (event := json.loads(line)).get("executable")
        )

    def setUp(self) -> None:
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.repo = self.root / "project"
        self.repo.mkdir()
        self.out = self.root / "out"
        self.env = {
            **os.environ,
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "Fixture",
            "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
            "GIT_COMMITTER_NAME": "Fixture",
            "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
        }
        self.git("init", "-q", "-b", "main")
        self.git("symbolic-ref", "refs/remotes/origin/HEAD", "refs/heads/main")
        (self.repo / ".gitattributes").write_text("blackhole.toml -merge\n")
        self.git("add", ".gitattributes")
        self.commit("legacy.test.ts", 'test.skip("legacy", () => {});\n')

    def git(self, *args: str) -> str:
        return subprocess.check_output(
            ["git", "-c", "core.hooksPath=/dev/null", *args],
            cwd=self.repo, env=self.env, text=True,
        ).strip()

    def commit(self, path: str, contents: str) -> str:
        (self.repo / path).write_text(contents)
        self.git("add", "--", path)
        self.git("commit", "-qm", path)
        return self.git("rev-parse", "HEAD")

    def merge(self, path: str, contents: str) -> tuple[str, str, str]:
        base = self.git("rev-parse", "HEAD")
        self.git("switch", "-qc", "change")
        proposed = self.commit(path, contents)
        self.git("switch", "-q", "main")
        self.git("merge", "-q", "--no-ff", "-m", path, "change")
        self.git("branch", "-d", "change")
        return self.git("rev-parse", "HEAD"), base, proposed

    def invoke_backtest(self, binary: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-I", ROOT / "tools/backtest.py", "--binary", binary,
             "--repo", self.repo, "--limit", "20", "--out", self.out, *args],
            cwd=ROOT, env=self.env, capture_output=True, text=True,
        )

    def run_backtest(self, binary: Path, *args: str) -> tuple[list[dict], dict]:
        before = self.git("status", "--porcelain"), self.git("worktree", "list", "--porcelain")
        result = self.invoke_backtest(binary, *args)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            (self.git("status", "--porcelain"), self.git("worktree", "list", "--porcelain")),
            before,
        )
        report = self.out / self.repo.name
        records = [json.loads(line) for line in (report / "results.jsonl").read_text().splitlines()]
        return records, json.loads((report / "summary.json").read_text())

    def test_refuses_output_or_report_inside_the_source_clone(self) -> None:
        original = "original tracked summary\n"
        self.commit("summary.json", original)
        self.merge("clean.ts", "export const value = 1;\n")
        self.out.mkdir()
        (self.out / self.repo.name).symlink_to(self.repo, target_is_directory=True)
        for output in (self.root, self.repo / "reports", self.out):
            with self.subTest(output=output):
                result = self.invoke_backtest(self.binary, "--out", str(output))
                self.assertEqual(
                    (result.returncode, (self.repo / "summary.json").read_text(),
                     self.git("status", "--porcelain")),
                    (1, original, ""),
                )
                self.assertIn("inside the source clone", result.stderr)

    def test_merge_backtest_baselines_existing_findings_and_counts_new_ones(self) -> None:
        clean = self.merge("clean.ts", "export const value = 1;\n")
        red = self.merge(
            "new.test.ts",
            'test.skip("new", () => { expect("word".length).toBe(4); });\n'
            'test.skip("more", () => { expect("words".length).toBe(5); });\n',
        )
        self.git("switch", "-q", "--detach", clean[0])
        (self.repo / "untracked").write_text("leave this here")
        records, summary = self.run_backtest(self.binary)
        self.assertEqual(
            [(row["commit"], row["base"], row["proposed"]) for row in records], [red, clean],
        )
        self.assertEqual(
            [(row["state"], row["exit_code"]) for row in records], [("red", 1), ("green", 0)],
        )
        self.assertEqual(records[0]["last_line"], "blackhole ci: red (findings)")
        self.assertEqual(records[0]["findings"], [
            {"path": "new.test.ts", "line": line, "rule": "ts/no-skipped-tests", "label": None}
            for line in (1, 2)
        ])
        self.assertEqual(records[1]["findings"], [])
        self.assertEqual(summary["judged"], 2)
        self.assertEqual(summary["skipped"], 0)
        self.assertEqual(summary["rules"], {"ts/no-skipped-tests": {"changes": 1, "flag_rate": 0.5}})

    def test_refuses_output_in_a_separate_git_directory(self) -> None:
        source = self.repo
        self.repo = self.root / "separate"
        storage = self.root / "storage"
        self.repo.mkdir()
        self.git("clone", "--quiet", "--separate-git-dir", str(storage), str(source), ".")
        output = storage / "reports"
        result = self.invoke_backtest(self.binary, "--out", str(output))
        self.assertEqual((result.returncode, output.exists()), (1, False))
        self.assertIn("inside the source clone", result.stderr)

    def test_refuses_report_symlink_into_a_linked_worktrees_git_directories(self) -> None:
        source = self.repo
        linked = self.root / "linked"
        self.git("worktree", "add", "--quiet", "--detach", str(linked))
        self.repo = linked
        self.out.mkdir()
        report = self.out / linked.name
        for directory in (source / ".git", source / ".git/worktrees/linked"):
            with self.subTest(directory=directory):
                output = directory / "reports"
                report.symlink_to(output, target_is_directory=True)
                result = self.invoke_backtest(self.binary)
                report.unlink()
                self.assertEqual((result.returncode, output.exists()), (1, False))
                self.assertIn("inside the source clone", result.stderr)

    def test_refuses_output_in_a_symlinked_object_directory(self) -> None:
        self.merge("clean.ts", "export const value = 1;\n")
        objects = self.repo / ".git/objects"
        external = self.root / "objects"
        objects.rename(external)
        objects.symlink_to(external, target_is_directory=True)
        output = external / "reports"

        result = self.invoke_backtest(self.binary, "--out", str(output))
        self.assertEqual((result.returncode, output.exists()), (1, False))
        self.assertIn("inside the source clone", result.stderr)

    def test_refuses_report_in_an_alternate_object_directory(self) -> None:
        self.merge("clean.ts", "export const value = 1;\n")
        external = self.root / "alternate\nobjects"
        external.mkdir()
        objects = self.repo / ".git/objects"
        (objects / "info/alternates").write_text('"../../../alternate\\nobjects"\n')
        self.out.mkdir()
        output = external / "reports"
        report = self.out / self.repo.name
        report.symlink_to(output, target_is_directory=True)

        result = self.invoke_backtest(self.binary)
        self.assertEqual((result.returncode, output.exists()), (1, False))
        self.assertIn("inside the source clone", result.stderr)

    def test_refuses_a_nested_source_repo(self) -> None:
        nested = self.repo / "nested"
        nested.mkdir()
        output = self.repo / "reports"
        result = self.invoke_backtest(self.binary, "--repo", str(nested), "--out", str(output))
        self.assertEqual((result.returncode, output.exists()), (1, False))
        self.assertIn("worktree root", result.stderr)

    def test_sparse_checkout_does_not_change_verdict(self) -> None:
        self.merge("clean.ts", "export const value = 1;\n")
        full = self.run_backtest(self.binary)
        self.assertEqual(full[0][0]["state"], "green")
        self.git(
            "sparse-checkout", "set", "--no-cone", "/clean.ts", "/blackhole.toml", "/.blackhole/",
        )
        self.assertFalse((self.repo / "legacy.test.ts").exists())
        self.assertEqual(self.run_backtest(self.binary), full)

    def test_replay_leaves_source_git_directory_unchanged(self) -> None:
        self.merge("clean.ts", "export const value = 1;\n")
        source_git = self.repo / ".git"
        marker = source_git / "filter-ran"
        (source_git / "info/attributes").write_text("legacy.test.ts filter=fixture\n")
        self.git("config", "filter.fixture.smudge", f"touch {shlex.quote(str(marker))} && cat")

        def git_files() -> dict[Path, bytes]:
            return {
                path.relative_to(source_git): path.read_bytes()
                for path in source_git.rglob("*") if path.is_file() and path.name != "index"
            }

        before = git_files()
        rows, _ = self.run_backtest(self.binary)
        self.assertEqual(rows[0]["state"], "green")
        self.assertFalse(marker.exists())
        self.assertEqual(git_files(), before)

    def test_findings_use_records_for_paths_that_look_like_a_gate(self) -> None:
        self.merge(
            "protected: new.test.ts",
            'test.skip("new", () => { expect("word".length).toBe(4); });\n',
        )

        rows, summary = self.run_backtest(self.binary)
        self.assertEqual((rows[0]["state"], rows[0]["gates"]), ("red", ["findings"]))
        self.assertEqual(rows[0]["findings"], [
            {"path": "protected: new.test.ts", "line": 1, "rule": "ts/no-skipped-tests",
             "label": None},
        ])
        self.assertEqual(summary["rules"], {"ts/no-skipped-tests": {"changes": 1, "flag_rate": 1.0}})

    def test_baseline_uses_only_the_files_committed_in_the_replay(self) -> None:
        self.commit("blackhole.toml", "[boundaries]\nmax-lines = 12\n")
        clean, _, _ = self.merge("clean.ts", "export const value = 1;\n")
        large, _, _ = self.merge("large.ts", "\n" * 12 + "export {};\n")

        rows, summary = self.run_backtest(self.binary)
        self.assertEqual(
            [(row["commit"], row["state"], row["gates"]) for row in rows],
            [(large, "red", ["findings"]), (clean, "green", [])],
        )
        self.assertEqual(rows[0]["findings"], [
            {"path": "large.ts", "line": 13, "rule": "boundary/file-size", "label": None},
        ])
        self.assertEqual(summary["rules"], {"boundary/file-size": {"changes": 1, "flag_rate": 0.5}})

    def test_baseline_with_a_finding_the_ledger_cannot_allow_still_judges_the_change(self) -> None:
        self.commit("blackhole.toml", "[boundaries]\nmax-lines = 12\n")
        self.commit("broken.ts", "export const = ;\n")
        clean, _, _ = self.merge("clean.ts", "export const value = 1;\n")

        rows, _ = self.run_backtest(self.binary)
        self.assertEqual(
            [(row["commit"], row["state"], row["gates"]) for row in rows],
            [(clean, "red", ["findings"])],
        )
        self.assertEqual([finding["path"] for finding in rows[0]["findings"]], ["broken.ts"])

    def test_baseline_row_filter_keeps_a_path_that_ends_like_a_generated_file(self) -> None:
        self.commit("blackhole.toml", "[boundaries]\nmax-lines = 12\n")
        self.commit("broken.ts", "export const = ;\n")
        (self.repo / "a\u2028.claude").mkdir()
        self.commit("a\u2028.claude/settings.json", "{}\n" + "\n" * 12)
        clean, _, _ = self.merge("clean.ts", "export const value = 1;\n")

        rows, _ = self.run_backtest(self.binary)
        self.assertEqual(
            [(row["commit"], row["state"], row["gates"]) for row in rows],
            [(clean, "red", ["findings"])],
        )

    def test_protected_gate_does_not_count_as_a_rule_finding(self) -> None:
        path = ".github/note\nnew.test.ts:1:2 ts/no-skipped-tests marker"
        (self.repo / path).parent.mkdir(parents=True)
        self.merge(path, "Protected change\n")
        rows, summary = self.run_backtest(self.binary)
        self.assertEqual(rows[0]["last_line"], "blackhole ci: red (protected)")
        self.assertEqual(summary["rules"], {})
        self.assertEqual(rows[0]["findings"], [])
        self.assertEqual(rows[0]["gates"], ["protected"])

    def test_coverage_gate_counts_as_coverage(self) -> None:
        config = ROOT / "evasion/coverage-changed/base/blackhole.toml"
        self.commit("blackhole.toml", config.read_text())
        self.merge("covered.ts", "export const value = 1;\n")

        rows, summary = self.run_backtest(self.binary)
        self.assertEqual((rows[0]["state"], rows[0]["gates"]), ("red", ["coverage"]))
        self.assertEqual(rows[0]["findings"], [])
        self.assertEqual(summary["rules"], {"coverage": {"changes": 1, "flag_rate": 1.0}})

    def test_judged_changes_count_even_with_an_empty_findings_file(self) -> None:
        self.merge("new.ts", "export const value = 1;\n")
        binary = self.root / "fake"
        lines = [
            "new.ts:1:2 ts/no-skipped-tests marker",
            "blackhole ci: red (findings)",
        ]
        binary.write_text(
            "#!/bin/sh\n"
            f'case "$1" in init|ledger) exec {shlex.quote(str(self.binary))} "$@";; esac\n'
            + "printf '%s\\n' " + shlex.join(lines) + "\nexit 1\n"
        )
        binary.chmod(0o700)

        rows, summary = self.run_backtest(binary)
        self.assertEqual((rows[0]["state"], rows[0]["exit_code"]), ("red", 1))
        self.assertEqual(rows[0]["gates"], ["findings"])
        self.assertEqual(rows[0]["findings"], [])
        self.assertEqual((summary["judged"], summary["states"]["red"]), (1, 1))
        self.assertEqual(summary["rules"], {})

    def test_objection_is_judged_on_the_original_commit_utc_date(self) -> None:
        self.env.update(
            GIT_AUTHOR_DATE="2001-01-11T23:30:00-02:00",
            GIT_COMMITTER_DATE="2001-02-10T23:30:00-02:00",
        )
        (self.repo / ".blackhole").mkdir()
        self.commit(
            ".blackhole/objections", "core/no-suppression\ta.py\t1\t2001-02-01\t-\treason\n",
        )
        example = ROOT / "evasion/expired/head/a.py"
        self.merge("a.py", example.read_text())
        rows, summary = self.run_backtest(self.binary)
        self.assertEqual(rows[0]["state"], "green")
        self.assertEqual(rows[0]["today"], "2001-02-11")
        self.assertEqual(summary["rules"], {})

    def test_squash_backtest_skips_conflicts_and_keeps_judging(self) -> None:
        clean = self.commit("clean.ts", "export const value = 1;\n")
        conflict = self.commit("blackhole.toml", 'exclude = ["legacy.test.ts"]\n')
        records, summary = self.run_backtest(self.binary, "--mode", "squash")
        self.assertEqual([row["commit"] for row in records], [conflict, clean])
        self.assertEqual(records[0]["state"], "skipped")
        self.assertIn("conflict", records[0]["reason"])
        self.assertIn("blackhole.toml", records[0]["reason"])
        self.assertEqual(records[1]["state"], "green")
        self.assertEqual(summary["judged"], 1)
        self.assertEqual(summary["skipped"], 1)

    def test_baseline_uses_the_date_of_a_future_dated_commit(self) -> None:
        tomorrow = (datetime.now(UTC).date() + timedelta(days=1)).isoformat()
        self.env.update(
            GIT_AUTHOR_DATE=f"{tomorrow}T12:00:00Z",
            GIT_COMMITTER_DATE=f"{tomorrow}T12:00:00Z",
        )
        (self.repo / ".blackhole").mkdir()
        self.commit(".blackhole/objections", f"core/no-suppression\ta.py\t1\t{tomorrow}\t-\treason\n")
        self.merge("a.py", (ROOT / "evasion/expired/head/a.py").read_text())

        rows, summary = self.run_backtest(self.binary)
        self.assertEqual((rows[0]["state"], rows[0]["today"]), ("green", tomorrow))
        self.assertEqual(summary["judged"], 1)

    def test_failed_baseline_is_reported_and_the_next_change_is_judged(self) -> None:
        clean, _, _ = self.merge("clean.ts", "export const value = 1;\n")
        (self.repo / ".blackhole").mkdir()
        self.commit(".blackhole/objections", "invalid\n")
        broken, _, _ = self.merge("later.ts", "export const value = 2;\n")

        rows, summary = self.run_backtest(self.binary)
        self.assertEqual(
            [(row["commit"], row["state"], row["exit_code"]) for row in rows],
            [(broken, "baseline_failure", 2), (clean, "green", 0)],
        )
        self.assertIn("objection", rows[0]["raw_output"])
        self.assertEqual(summary["states"]["baseline_failure"], 1)
        self.assertEqual(summary["judged"], 1)

    def test_nonverdicts_use_isolated_home_and_do_not_count_findings(self) -> None:
        (self.repo / ".claude").mkdir()
        self.commit(".claude/settings.json", '{"permissions":{}}\n')
        self.merge("waiting.ts", "export const waiting = true;\n")
        self.merge("usage.ts", "export const usage = 1;\n")
        self.merge("busy.ts", "export const busy = 2;\n")
        binary = self.root / "fake"
        log = self.root / "homes"
        original_home = self.root / "original-home"
        original_home.mkdir()
        self.env.update(HOME=str(original_home), XDG_CONFIG_HOME=str(original_home))
        binary.write_text(
            "#!/bin/sh\n"
            f'printf "%s\\n" "$HOME" "$XDG_CONFIG_HOME" >> {shlex.quote(str(log))}\n'
            f'case "$1" in init|ledger) exec {shlex.quote(str(self.binary))} "$@";; esac\n'
            """[ "$(cat .claude/settings.json)" = '{"permissions":{}}' ] || exit 91\n"""
            "[ ! -e .codex/hooks.json ] || exit 92\n"
            'paths=$(git diff --name-only "$3^" "$3") || exit 93\n'
            '[ "$paths" = "$(printf "%s\\n" .blackhole/ledger blackhole.toml)" ] || exit 94\n'
            'printf "%s\\n" "busy.ts:1:2 ts/no-skipped-tests" "no usable verdict"\n'
            'if [ -f busy.ts ]; then exit 75; fi\n'
            'if [ -f usage.ts ]; then exit 2; fi\n'
            'exit 3\n'
        )
        binary.chmod(0o700)
        records, summary = self.run_backtest(binary)
        self.assertEqual(
            [(row["state"], row["exit_code"]) for row in records],
            [("no_verdict", 75), ("error", 2), ("waiting", 3)],
        )
        for row in records[:2]:
            self.assertEqual(row["last_line"], "no usable verdict")
            self.assertEqual(row["findings"], [])
        self.assertEqual(summary["judged"], 0)
        self.assertEqual(summary["states"]["no_verdict"], 1)
        self.assertEqual(summary["states"]["error"], 1)
        self.assertEqual(summary["states"]["waiting"], 1)
        self.assertEqual(summary["rules"], {})
        homes = log.read_text().splitlines()
        self.assertEqual(len(homes), 18)
        self.assertTrue(all(Path(home).is_relative_to(self.out) for home in homes), homes)
        self.assertEqual(list(original_home.iterdir()), [])
