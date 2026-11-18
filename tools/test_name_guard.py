from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

GUARD = Path(__file__).with_name("name_guard.py")
WORDS = "quokka\nzebrafish\n"


def run(
    args: list[str],
    cwd: Path,
    banned: str | None = WORDS,
    stdin: str = "",
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    full = {
        "PATH": os.environ["PATH"],
        "HOME": str(cwd),
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_CONFIG_SYSTEM": os.devnull,
        "BLACKHOLE_BANNED_FILE": str(cwd / "no-such-list"),
        **(env or {}),
    }
    if banned is not None:
        full["BLACKHOLE_BANNED"] = banned
    return subprocess.run(
        [sys.executable, "-I", str(GUARD), *args],
        cwd=cwd,
        env=full,
        input=stdin,
        capture_output=True,
        text=True,
        check=False,
    )


def git(cwd: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=True,
        env={
            "PATH": os.environ["PATH"],
            "HOME": str(cwd),
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_SYSTEM": os.devnull,
        },
    )
    return done.stdout.strip()


class Repo(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        git(self.dir, "init", "-q", "-b", "main")

    def write(self, name: str, text: str) -> None:
        path = self.dir / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def commit(self, message: str) -> str:
        git(self.dir, "add", "-A")
        git(self.dir, "commit", "-q", "--allow-empty", "-m", message)
        return git(self.dir, "rev-parse", "HEAD")


class Text(Repo):
    def check(self, text: str) -> subprocess.CompletedProcess[str]:
        return run(["text"], self.dir, env={"GUARD_TEXT_SAMPLE": text})

    def test_clean_text_passes(self) -> None:
        done = self.check("a quiet koala and a zebra\n")
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_variants_are_caught(self) -> None:
        variants = [
            "quokka",
            "Quokka",
            "QUOKKA",
            "quo kka",
            "q u o k k a",
            "quo-kka",
            "quo_kka",
            "quo.kka",
            "quo\nkka",
            "quo​kka",
            "the QuOkKaHandler class",
            "zebra fish",
            "Zebra-Fish",
            "ｑｕｏｋｋａ",
        ]
        for variant in variants:
            with self.subTest(variant=variant):
                self.assertEqual(self.check(f"x {variant} y").returncode, 1)

    def test_output_never_prints_the_match(self) -> None:
        done = self.check("line one\nhello QuOkKa there\n")
        self.assertEqual(done.returncode, 1)
        both = (done.stdout + done.stderr).lower()
        self.assertNotIn("quokka", both)
        self.assertNotIn("hello", both)
        self.assertIn("SAMPLE:2", done.stderr)


class FailClosed(Repo):
    def test_missing_list_fails(self) -> None:
        done = run(["text"], self.dir, banned=None)
        self.assertEqual(done.returncode, 2)

    def test_empty_list_fails(self) -> None:
        done = run(["text"], self.dir, banned=" \n\n")
        self.assertEqual(done.returncode, 2)

    def test_short_word_fails(self) -> None:
        done = run(["text"], self.dir, banned="ab\n")
        self.assertEqual(done.returncode, 2)

    def test_list_is_read_from_the_file(self) -> None:
        listed = self.dir / "list"
        listed.write_text(WORDS, encoding="utf-8")
        done = run(
            ["text"],
            self.dir,
            banned=None,
            env={"BLACKHOLE_BANNED_FILE": str(listed), "GUARD_TEXT_SAMPLE": "Quokka"},
        )
        self.assertEqual(done.returncode, 1)


class Staged(Repo):
    def test_clean_stage_passes(self) -> None:
        self.write("a.txt", "fine\n")
        git(self.dir, "add", "-A")
        self.assertEqual(run(["staged"], self.dir).returncode, 0)

    def test_staged_content_is_caught(self) -> None:
        self.write("a.txt", "fine\nnot fine: Quokka\n")
        git(self.dir, "add", "-A")
        done = run(["staged"], self.dir)
        self.assertEqual(done.returncode, 1)
        self.assertIn("a.txt:2", done.stderr)

    def test_staged_file_name_is_caught(self) -> None:
        self.write("src/quokka_client.rs", "fine\n")
        git(self.dir, "add", "-A")
        done = run(["staged"], self.dir)
        self.assertEqual(done.returncode, 1)
        self.assertNotIn("quokka", (done.stdout + done.stderr).lower())
        self.assertIn("src/[banned]_client.rs", done.stderr)

    def test_utf16_content_is_caught(self) -> None:
        (self.dir / "a.bin").write_bytes("Quokka".encode("utf-16"))
        git(self.dir, "add", "-A")
        self.assertEqual(run(["staged"], self.dir).returncode, 1)

    def test_branch_name_is_caught(self) -> None:
        self.write("a.txt", "fine\n")
        git(self.dir, "add", "-A")
        git(self.dir, "checkout", "-q", "-b", "fix/quokka-sync")
        self.assertEqual(run(["staged"], self.dir).returncode, 1)

    def test_deleting_a_badly_named_file_is_allowed(self) -> None:
        self.write("quokka.txt", "fine\n")
        self.commit("chore: start")
        git(self.dir, "rm", "-q", "quokka.txt")
        self.assertEqual(run(["staged"], self.dir).returncode, 0)

    def test_unstaged_content_is_ignored(self) -> None:
        self.write("a.txt", "fine\n")
        git(self.dir, "add", "-A")
        self.write("a.txt", "Quokka\n")
        self.assertEqual(run(["staged"], self.dir).returncode, 0)


class OddPaths(Repo):
    def setUp(self) -> None:
        super().setUp()
        Path(self.dir, os.fsdecode(b"caf\xe9.txt")).write_bytes(b"Quokka\n")

    def test_staged_content_under_a_non_utf8_name_is_caught(self) -> None:
        git(self.dir, "add", "-A")
        self.assertEqual(run(["staged"], self.dir).returncode, 1)

    def test_committed_content_under_a_non_utf8_name_is_caught(self) -> None:
        head = self.commit("feat: add")
        self.assertEqual(run(["range", "0" * 40, head], self.dir).returncode, 1)
        self.assertEqual(run(["tree"], self.dir).returncode, 1)


class Message(Repo):
    def test_commit_message_is_caught(self) -> None:
        self.write("msg", "fix: sync with zebra fish\n")
        self.assertEqual(run(["message", "msg"], self.dir).returncode, 1)

    def test_clean_message_passes(self) -> None:
        self.write("msg", "fix: sync\n")
        self.assertEqual(run(["message", "msg"], self.dir).returncode, 0)


class Range(Repo):
    def test_clean_range_passes(self) -> None:
        base = self.commit("chore: start")
        self.write("a.txt", "fine\n")
        head = self.commit("feat: add a")
        self.assertEqual(run(["range", base, head], self.dir).returncode, 0)

    def test_content_removed_later_is_still_caught(self) -> None:
        base = self.commit("chore: start")
        self.write("a.txt", "Quokka\n")
        self.commit("feat: add a")
        self.write("a.txt", "fine\n")
        head = self.commit("fix: clean a")
        self.assertEqual(run(["range", base, head], self.dir).returncode, 1)

    def test_a_blob_repeated_across_commits_is_reported_once(self) -> None:
        base = self.commit("chore: start")
        self.write("a.txt", "Quokka\n")
        self.commit("feat: add a")
        git(self.dir, "mv", "a.txt", "b.txt")
        head = self.commit("refactor: move a")
        done = run(["range", base, head], self.dir)
        self.assertEqual(done.returncode, 1)
        self.assertEqual(done.stderr.count("banned name #1"), 1)

    def test_message_in_range_is_caught(self) -> None:
        base = self.commit("chore: start")
        head = self.commit("fix: quokka outage")
        self.assertEqual(run(["range", base, head], self.dir).returncode, 1)

    def test_author_in_range_is_caught(self) -> None:
        base = self.commit("chore: start")
        git(self.dir, "commit", "-q", "--allow-empty", "-m", "x", "--author", "Q <dev@quokka.example>")
        head = git(self.dir, "rev-parse", "HEAD")
        self.assertEqual(run(["range", base, head], self.dir).returncode, 1)

    def test_zero_base_scans_all_history(self) -> None:
        self.write("a.txt", "Quokka\n")
        self.commit("feat: add a")
        self.write("a.txt", "fine\n")
        head = self.commit("fix: clean a")
        self.assertEqual(run(["range", "0" * 40, head], self.dir).returncode, 1)

    def test_unknown_base_scans_all_history(self) -> None:
        self.write("a.txt", "Quokka\n")
        self.commit("feat: add a")
        self.write("a.txt", "fine\n")
        head = self.commit("fix: clean a")
        self.assertEqual(run(["range", "1" * 40, head], self.dir).returncode, 1)

    def test_env_text_is_scanned_with_the_range(self) -> None:
        base = self.commit("chore: start")
        head = self.commit("feat: more")
        done = run(["range", base, head], self.dir, env={"GUARD_TEXT_PR_TITLE": "Quokka fix"})
        self.assertEqual(done.returncode, 1)
        self.assertIn("PR_TITLE:1", done.stderr)


class Tree(Repo):
    def test_tree_scans_tracked_files_and_names(self) -> None:
        self.write("a.txt", "fine\n")
        self.commit("chore: start")
        self.assertEqual(run(["tree"], self.dir).returncode, 0)
        self.write("docs/zebra-fish.md", "fine\n")
        self.commit("docs: add")
        self.assertEqual(run(["tree"], self.dir).returncode, 1)


class Push(Repo):
    def test_new_branch_push_scans_its_commits(self) -> None:
        self.write("a.txt", "Quokka\n")
        head = self.commit("feat: add a")
        line = f"refs/heads/main {head} refs/heads/main {'0' * 40}\n"
        self.assertEqual(run(["push"], self.dir, stdin=line).returncode, 1)

    def test_clean_push_passes(self) -> None:
        self.write("a.txt", "fine\n")
        head = self.commit("feat: add a")
        line = f"refs/heads/main {head} refs/heads/main {'0' * 40}\n"
        self.assertEqual(run(["push"], self.dir, stdin=line).returncode, 0)

    def test_pushed_ref_name_is_caught(self) -> None:
        head = self.commit("chore: start")
        line = f"refs/heads/x {head} refs/heads/quokka-fix {'0' * 40}\n"
        self.assertEqual(run(["push"], self.dir, stdin=line).returncode, 1)

    def test_annotated_tag_message_is_caught(self) -> None:
        self.commit("chore: start")
        git(self.dir, "tag", "-a", "v1", "-m", "release for Quokka")
        tag = git(self.dir, "rev-parse", "refs/tags/v1")
        line = f"refs/tags/v1 {tag} refs/tags/v1 {'0' * 40}\n"
        self.assertEqual(run(["push"], self.dir, stdin=line).returncode, 1)

    def test_branch_delete_is_skipped(self) -> None:
        self.commit("chore: start")
        line = f"(delete) {'0' * 40} refs/heads/old {'1' * 40}\n"
        self.assertEqual(run(["push"], self.dir, stdin=line).returncode, 0)


if __name__ == "__main__":
    unittest.main()
