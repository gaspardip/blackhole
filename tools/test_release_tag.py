import tempfile
import unittest
from pathlib import Path

from test_name_guard import git
from release_tag import verify_tag


class ReleaseTag(unittest.TestCase):
    def setUp(self) -> None:
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.repo = Path(temp.name)
        git(self.repo, "init", "-q", "-b", "main")
        self.version("1.2.3-rc.1")
        self.commit()
        git(self.repo, "tag", "-a", "v1.2.3-rc.1", "-m", "prerelease")

    def version(self, version: str) -> None:
        (self.repo / "Cargo.toml").write_text(
            f'[package]\nname = "blackhole-check"\nversion = "{version}"\n'
        )
        (self.repo / "Cargo.lock").write_text(
            f'version = 4\n\n[[package]]\nname = "blackhole-check"\nversion = "{version}"\n'
        )

    def commit(self) -> None:
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "--allow-empty", "-qm", "chore: release version")

    def final(self) -> None:
        self.commit()
        git(self.repo, "tag", "v1.2.3")
        git(self.repo, "update-ref", "refs/remotes/origin/main", "HEAD")

    def check(self, tag: str = "v1.2.3", released: bool = True) -> str:
        return verify_tag(self.repo, tag, lambda rc: released and rc == "v1.2.3-rc.1")

    def test_final_accepts_only_version_changes(self) -> None:
        self.version("1.2.3")
        self.final()
        self.assertEqual(self.check(), "latest")

    def test_final_rejects_an_extra_line(self) -> None:
        self.version("1.2.3")
        with (self.repo / "Cargo.lock").open("a") as lock:
            lock.write("# extra change\n")
        self.final()
        with self.assertRaisesRegex(ValueError, "version-only"):
            self.check()

    def test_final_rejects_an_extra_file(self) -> None:
        self.version("1.2.3")
        (self.repo / "other").touch()
        self.final()
        with self.assertRaisesRegex(ValueError, "version-only"):
            self.check()

    def test_final_rejects_a_release_from_another_history(self) -> None:
        git(self.repo, "checkout", "--orphan", "other")
        self.version("1.2.3")
        self.final()
        with self.assertRaisesRegex(ValueError, "version-only"):
            self.check()

    def test_final_requires_a_published_github_prerelease(self) -> None:
        self.version("1.2.3")
        self.final()
        with self.assertRaisesRegex(ValueError, "version-only"):
            self.check(released=False)

    def test_matching_rc_tag_selects_next(self) -> None:
        git(self.repo, "update-ref", "refs/remotes/origin/main", "HEAD")
        self.assertEqual(self.check("v1.2.3-rc.1", released=False), "next")

    def test_rc_tag_must_match_the_manifest(self) -> None:
        git(self.repo, "tag", "v1.2.3-rc.2")
        git(self.repo, "update-ref", "refs/remotes/origin/main", "HEAD")
        with self.assertRaisesRegex(ValueError, "Cargo version"):
            self.check("v1.2.3-rc.2")

    def test_rc_tag_must_be_on_main(self) -> None:
        git(self.repo, "checkout", "--orphan", "other")
        (self.repo / "different-tree").touch()
        self.commit()
        git(self.repo, "update-ref", "refs/remotes/origin/main", "HEAD")
        git(self.repo, "checkout", "--detach", "v1.2.3-rc.1")
        with self.assertRaisesRegex(ValueError, "origin/main"):
            self.check("v1.2.3-rc.1")
