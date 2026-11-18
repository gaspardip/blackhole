from __future__ import annotations

import json
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
        self.assertEqual(run(["push", "origin"], self.dir, stdin=line).returncode, 1)

    def test_clean_push_passes(self) -> None:
        self.write("a.txt", "fine\n")
        head = self.commit("feat: add a")
        line = f"refs/heads/main {head} refs/heads/main {'0' * 40}\n"
        self.assertEqual(run(["push", "origin"], self.dir, stdin=line).returncode, 0)

    def test_pushed_ref_name_is_caught(self) -> None:
        head = self.commit("chore: start")
        line = f"refs/heads/x {head} refs/heads/quokka-fix {'0' * 40}\n"
        self.assertEqual(run(["push", "origin"], self.dir, stdin=line).returncode, 1)

    def test_annotated_tag_message_is_caught(self) -> None:
        self.commit("chore: start")
        git(self.dir, "tag", "-a", "v1", "-m", "release for Quokka")
        tag = git(self.dir, "rev-parse", "refs/tags/v1")
        line = f"refs/tags/v1 {tag} refs/tags/v1 {'0' * 40}\n"
        self.assertEqual(run(["push", "origin"], self.dir, stdin=line).returncode, 1)

    def test_a_tag_inside_a_tag_is_caught(self) -> None:
        self.commit("chore: start")
        git(self.dir, "tag", "-a", "inner", "-m", "release for Quokka")
        git(self.dir, "tag", "-a", "outer", "-m", "fine", "refs/tags/inner")
        tag = git(self.dir, "rev-parse", "refs/tags/outer")
        line = f"refs/tags/outer {tag} refs/tags/outer {'0' * 40}\n"
        self.assertEqual(run(["push", "origin"], self.dir, stdin=line).returncode, 1)

    def test_only_the_pushed_remote_counts_as_already_published(self) -> None:
        self.write("a.txt", "Quokka\n")
        head = self.commit("feat: add a")
        line = f"refs/heads/main {head} refs/heads/new {'0' * 40}\n"
        git(self.dir, "update-ref", "refs/remotes/other/main", head)
        self.assertEqual(run(["push", "origin"], self.dir, stdin=line).returncode, 1)
        git(self.dir, "update-ref", "refs/remotes/origin/main", head)
        self.assertEqual(run(["push", "origin"], self.dir, stdin=line).returncode, 0)

    def test_branch_delete_is_skipped(self) -> None:
        self.commit("chore: start")
        line = f"(delete) {'0' * 40} refs/heads/old {'1' * 40}\n"
        self.assertEqual(run(["push", "origin"], self.dir, stdin=line).returncode, 0)


class Event(Repo):
    def setUp(self) -> None:
        super().setUp()
        self.base = self.commit("chore: start")

    def event(
        self, name: str, payload: dict[str, object], **env: str
    ) -> subprocess.CompletedProcess[str]:
        path = self.dir / ".git" / "event.json"
        repository = {"full_name": "o/r", "owner": {"type": "User"}}
        path.write_text(json.dumps({"repository": repository} | payload), encoding="utf-8")
        env |= {"GITHUB_EVENT_NAME": name, "GITHUB_EVENT_PATH": str(path)}
        return run(["event"], self.dir, env=env)

    def pull_request(
        self,
        title: str = "fix: sync",
        body: str | None = None,
        association: str = "OWNER",
        head_repo: str | None = "o/r",
        head: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        pull = {
            "title": title,
            "body": body,
            "author_association": association,
            "head": {
                "ref": "fix/sync",
                "sha": head or self.base,
                "repo": {"full_name": head_repo} if head_repo else None,
            },
            "base": {"sha": self.base},
        }
        return self.event("pull_request_target", {"pull_request": pull})

    def test_clean_pull_request_passes(self) -> None:
        self.assertEqual(self.pull_request().returncode, 0)

    def test_pull_request_title_is_caught_without_printing_it(self) -> None:
        done = self.pull_request(title="Quokka fix")
        self.assertEqual(done.returncode, 1)
        self.assertNotIn("quokka", (done.stdout + done.stderr).lower())

    def test_pull_request_body_is_caught(self) -> None:
        self.assertEqual(self.pull_request(body="for Quokka").returncode, 1)

    def test_pull_request_commits_are_caught(self) -> None:
        self.write("a.txt", "Quokka\n")
        head = self.commit("feat: add a")
        self.assertEqual(self.pull_request(head=head).returncode, 1)

    def test_a_strangers_pull_request_gives_no_signal(self) -> None:
        for association in ["NONE", "CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR", "MEMBER"]:
            with self.subTest(association=association):
                done = self.pull_request(title="Quokka?", association=association)
                self.assertEqual(done.returncode, 0)

    def test_a_head_outside_the_repository_gives_no_signal(self) -> None:
        self.write("a.txt", "Quokka\n")
        head = self.commit("feat: add a")
        for head_repo in ["stranger/r", None]:
            with self.subTest(head_repo=head_repo):
                done = self.pull_request(title="Quokka", head_repo=head_repo, head=head)
                self.assertEqual(done.returncode, 0)

    def test_issue_text_is_caught_for_a_trusted_author_only(self) -> None:
        for association, expected in [("OWNER", 1), ("COLLABORATOR", 1), ("NONE", 0)]:
            with self.subTest(association=association):
                issue = {"title": "bug", "body": "seen at Quokka", "author_association": association}
                self.assertEqual(self.event("issues", {"issue": issue}).returncode, expected)

    def test_comment_is_scanned_without_the_issue_it_is_on(self) -> None:
        issue = {"title": "Quokka?", "body": "", "author_association": "NONE"}

        def comment(body: str, association: str) -> int:
            said = {"body": body, "author_association": association}
            return self.event("issue_comment", {"issue": issue, "comment": said}).returncode

        self.assertEqual(comment("thanks", "OWNER"), 0)
        self.assertEqual(comment("yes, Quokka", "OWNER"), 1)
        self.assertEqual(comment("is it Quokka?", "NONE"), 0)

    def test_release_text_is_caught(self) -> None:
        release = {"name": "v1", "tag_name": "v1", "body": "thanks Quokka"}
        self.assertEqual(self.event("release", {"release": release}).returncode, 1)

    def test_pushed_commits_and_ref_are_caught(self) -> None:
        self.write("a.txt", "Quokka\n")
        head = self.commit("feat: add a")
        clean = {"ref": "refs/heads/main", "before": self.base, "after": self.base}
        self.assertEqual(self.event("push", clean).returncode, 0)
        self.assertEqual(self.event("push", clean | {"after": head}).returncode, 1)
        self.assertEqual(self.event("push", clean | {"ref": "refs/heads/quokka"}).returncode, 1)

    def test_pushed_annotated_tag_message_is_caught(self) -> None:
        git(self.dir, "tag", "-a", "v1", "-m", "release for Quokka")
        tag = git(self.dir, "rev-parse", "refs/tags/v1")
        payload = {"ref": "refs/tags/v1", "before": "0" * 40, "after": tag}
        self.assertEqual(self.event("push", payload).returncode, 1)

    def test_ref_deletion_passes(self) -> None:
        payload = {"ref": "refs/heads/old", "before": self.base, "after": "0" * 40}
        self.assertEqual(self.event("push", payload).returncode, 0)

    def test_a_scheduled_run_scans_every_ref(self) -> None:
        self.assertEqual(self.event("schedule", {}).returncode, 0)
        git(self.dir, "tag", "quokka-1")
        self.assertEqual(self.event("schedule", {}).returncode, 1)
        git(self.dir, "tag", "-d", "quokka-1")
        git(self.dir, "checkout", "-q", "-b", "side")
        self.write("a.txt", "Quokka\n")
        self.commit("feat: add a")
        git(self.dir, "checkout", "-q", "-")
        self.assertEqual(self.event("workflow_dispatch", {}).returncode, 1)

    def test_an_organization_repository_fails_closed(self) -> None:
        issue = {"title": "bug", "body": None, "author_association": "MEMBER"}
        payload = {"repository": {"full_name": "o/r", "owner": {"type": "Organization"}}}
        self.assertEqual(self.event("issues", payload | {"issue": issue}).returncode, 2)

    def test_a_field_of_the_wrong_type_fails_closed(self) -> None:
        owner = {"title": "bug", "body": None, "author_association": "OWNER"}
        for issue in [owner | {"author_association": None}, owner | {"body": []}, owner | {"title": 1}]:
            with self.subTest(issue=issue):
                self.assertEqual(self.event("issues", {"issue": issue}).returncode, 2)
        push = {"ref": "refs/heads/old", "before": self.base, "after": "0" * 40}
        for payload in [push | {"after": "main"}, push | {"before": None}, {"ref": push["ref"], "after": push["after"]}]:
            with self.subTest(payload=payload):
                self.assertEqual(self.event("push", payload).returncode, 2)

    def test_an_event_without_a_scan_fails_closed(self) -> None:
        self.assertEqual(self.event("fork", {}).returncode, 2)

    def test_environment_text_is_not_scanned_for_an_event(self) -> None:
        issue = {"title": "bug", "body": None, "author_association": "NONE"}
        done = self.event("issues", {"issue": issue}, GUARD_TEXT_ISSUE="Quokka")
        self.assertEqual(done.returncode, 0)

    def test_an_unexpected_payload_fails_closed(self) -> None:
        self.assertEqual(self.event("issues", {}).returncode, 2)
        self.assertEqual(self.event("issues", {"issue": "Quokka"}).returncode, 2)
        self.assertEqual(run(["event"], self.dir).returncode, 2)


class Errors(Repo):
    def test_a_git_failure_does_not_echo_its_input(self) -> None:
        self.commit("chore: start")
        done = run(["range", "0" * 40, "quokka-missing"], self.dir)
        self.assertEqual(done.returncode, 2)
        self.assertNotIn("quokka", (done.stdout + done.stderr).lower())

    def test_an_unexpected_failure_is_not_reported_as_a_found_name(self) -> None:
        done = run(["push", "origin"], self.dir, stdin="quokka\n")
        self.assertEqual(done.returncode, 2)
        self.assertNotIn("quokka", (done.stdout + done.stderr).lower())


class Replaced(Repo):
    def test_a_replacement_ref_does_not_hide_a_commit(self) -> None:
        base = self.commit("chore: start")
        self.write("a.txt", "Quokka\n")
        head = self.commit("feat: add a")
        clean = git(self.dir, "commit-tree", f"{base}^{{tree}}", "-p", base, "-m", "feat: add a")
        git(self.dir, "replace", head, clean)
        self.assertEqual(run(["range", base, head], self.dir).returncode, 1)


class HiddenSubmodules(Repo):
    def setUp(self) -> None:
        super().setUp()
        self.base = self.commit("chore: start")
        git(self.dir, "update-index", "--add", "--cacheinfo", f"160000,{self.base},quokka-mod")

    def test_a_staged_submodule_name_is_caught(self) -> None:
        git(self.dir, "config", "diff.ignoreSubmodules", "all")
        self.assertEqual(run(["staged"], self.dir).returncode, 1)

    def test_a_committed_submodule_name_is_caught(self) -> None:
        self.write(".gitmodules", '[submodule "m"]\n\tpath = quokka-mod\n\turl = ./m\n\tignore = all\n')
        git(self.dir, "commit", "-q", "--no-verify", "-m", "feat: add a module")
        self.assertEqual(run(["range", self.base, "HEAD"], self.dir, banned="zebrafish").returncode, 0)
        self.assertEqual(run(["range", self.base, "HEAD"], self.dir).returncode, 1)


if __name__ == "__main__":
    unittest.main()
