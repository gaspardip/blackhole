import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.parse import parse_qs, urlsplit

from test_name_guard import git
from release_tag import github_candidate_succeeded, verify_tag


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
        return verify_tag(self.repo, tag, lambda rc, sha: released and rc == "v1.2.3-rc.1")

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

    def test_final_requires_a_successfully_released_candidate(self) -> None:
        self.version("1.2.3")
        self.final()
        with self.assertRaisesRegex(ValueError, "version-only"):
            self.check(released=False)

    def test_final_checks_successful_candidate_push_runs(self) -> None:
        rc = "v1.2.3-rc.1"
        sha = git(self.repo, "rev-parse", "HEAD").strip()
        self.version("1.2.3")
        self.final()
        run = {"head_branch": rc, "head_sha": sha, "event": "push", "conclusion": "success"}

        with patch.dict("os.environ", {
            "GITHUB_API_URL": "https://api.github.test",
            "GITHUB_REPOSITORY": "owner/project",
            "GH_TOKEN": "test-token",
        }), patch("release_tag.urlopen") as lookup:
            response = lookup.return_value.__enter__
            response.return_value = io.BytesIO(json.dumps({"workflow_runs": [run]}).encode())
            self.assertEqual(verify_tag(self.repo, "v1.2.3", github_candidate_succeeded), "latest")
            request = urlsplit(lookup.call_args.args[0].full_url)
            self.assertEqual(request.path, "/repos/owner/project/actions/workflows/release.yml/runs")
            query = parse_qs(request.query)
            self.assertEqual(query["event"], ["push"])
            self.assertEqual(query["head_sha"], [sha])
            self.assertEqual(query["status"], ["success"])

            for runs in ([], [run | {"head_branch": "main"}], [run | {"head_sha": "0" * 40}]):
                with self.subTest(runs=runs), self.assertRaisesRegex(ValueError, "version-only"):
                    response.return_value = io.BytesIO(json.dumps({"workflow_runs": runs}).encode())
                    verify_tag(self.repo, "v1.2.3", github_candidate_succeeded)

            lookup.side_effect = [
                io.BytesIO(json.dumps({"workflow_runs": [run | {"head_branch": "main"}] * 100}).encode()),
                io.BytesIO(json.dumps({"workflow_runs": [run]}).encode()),
            ]
            self.assertEqual(verify_tag(self.repo, "v1.2.3", github_candidate_succeeded), "latest")
            self.assertEqual(parse_qs(urlsplit(lookup.call_args.args[0].full_url).query)["page"], ["2"])

            for status in (404, 503):
                with self.subTest(status=status), self.assertRaisesRegex(ValueError, f"HTTP {status}"):
                    error = HTTPError("", status, "", {}, None)
                    self.addCleanup(error.close)
                    lookup.side_effect = error
                    verify_tag(self.repo, "v1.2.3", github_candidate_succeeded)

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
