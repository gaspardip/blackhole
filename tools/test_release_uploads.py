import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError

from release_uploads import publish_github, registry_uploads

TAG = "v1.2.3-rc.1"
SHA256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
INTEGRITY = "sha512-3a81oZNherrMQXNJriBBMRLm+k6JqX6iCp7u5ktV05ohkpkqJ0/BqDa6PCOj/uu9RU1EI2Q86A4qmslPpUyknw=="
ENV = {
    "GITHUB_API_URL": "https://api.github.test",
    "GITHUB_REPOSITORY": "owner/project",
    "GH_TOKEN": "test-token",
    "NPM_DIST_TAG": "next",
    "RELEASE_NOTES": "Candidate",
}


def response(payload: object) -> io.BytesIO:
    return io.BytesIO(json.dumps(payload).encode())


class ReleaseUploads(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.npm = Path(tmp.name) / "candidate.tgz"
        self.crate = Path(tmp.name) / "candidate.crate"
        self.sums = Path(tmp.name) / "SHA256SUMS"
        for path in (self.npm, self.crate):
            path.write_bytes(b"abc")
        self.sums.write_text(f"{SHA256}  {self.crate.name}\n{SHA256}  {self.npm.name}\n")

    def http_error(self, status: int) -> HTTPError:
        error = HTTPError("", status, "", {}, io.BytesIO())
        self.addCleanup(error.close)
        return error

    def test_registry_uploads_skip_matching_versions_without_credentials(self) -> None:
        with patch("release_uploads.urlopen") as lookup:
            lookup.side_effect = [
                response({"dist": {"integrity": INTEGRITY}}),
                response({"version": {"checksum": SHA256}}),
            ]
            self.assertEqual(registry_uploads(TAG, self.npm, self.crate), {"npm": False, "crate": False})
            requests = [call.args[0] for call in lookup.call_args_list]
            self.assertEqual([request.full_url for request in requests], [
                "https://registry.npmjs.org/@gaspardip%2Fblackhole/1.2.3-rc.1",
                "https://crates.io/api/v1/crates/blackhole-check/1.2.3-rc.1",
            ])
            for request in requests:
                self.assertEqual(request.get_method(), "GET")
                self.assertIsNone(request.get_header("Authorization"))

    def test_registry_uploads_only_missing_versions(self) -> None:
        with patch("release_uploads.urlopen") as lookup:
            lookup.side_effect = [response({"dist": {"integrity": INTEGRITY}}), self.http_error(404)]
            self.assertEqual(registry_uploads(TAG, self.npm, self.crate), {"npm": False, "crate": True})
            lookup.side_effect = [self.http_error(404), response({"version": {"checksum": SHA256}})]
            self.assertEqual(registry_uploads(TAG, self.npm, self.crate), {"npm": True, "crate": False})

    def test_registry_uploads_refuse_digest_mismatch(self) -> None:
        with patch("release_uploads.urlopen") as lookup:
            lookup.side_effect = [
                response({"dist": {"integrity": "sha512-wrong"}}),
                response({"version": {"checksum": SHA256}}),
            ]
            with self.assertRaisesRegex(ValueError, "npm.*digest"):
                registry_uploads(TAG, self.npm, self.crate)
            lookup.side_effect = [self.http_error(404), response({"version": {"checksum": "wrong"}})]
            with self.assertRaisesRegex(ValueError, "crate.*digest"):
                registry_uploads(TAG, self.npm, self.crate)

    def test_registry_uploads_fail_closed_on_lookup_errors(self) -> None:
        with patch("release_uploads.urlopen") as lookup:
            lookup.side_effect = [self.http_error(503)]
            with self.assertRaises(HTTPError):
                registry_uploads(TAG, self.npm, self.crate)
            lookup.side_effect = [self.http_error(404), self.http_error(403)]
            with self.assertRaises(HTTPError):
                registry_uploads(TAG, self.npm, self.crate)

    def test_github_creates_a_draft_and_uploads_missing_assets(self) -> None:
        with patch.dict("os.environ", ENV), patch("release_uploads.urlopen", return_value=response([])), \
             patch("release_uploads.subprocess.run") as gh:
            publish_github(TAG, self.npm, self.crate, self.sums)
        commands = [call.args[0] for call in gh.call_args_list]
        self.assertEqual([command[2] for command in commands], ["create", "upload", "edit"])
        self.assertIn("--draft", commands[0])
        self.assertIn("--prerelease", commands[0])
        self.assertEqual(commands[1][-3:], [str(self.npm), str(self.crate), str(self.sums)])
        self.assertEqual(commands[2][-5:], ["--draft=false", "--title", TAG, "--notes", "Candidate"])

    def test_github_resumes_a_draft_without_reuploading_matching_assets(self) -> None:
        asset = {"name": self.crate.name, "digest": f"sha256:{SHA256}"}
        draft = {
            "tag_name": TAG, "draft": True, "prerelease": True, "assets": [asset],
            "name": "Edited title", "body": "Edited notes",
        }
        with patch.dict("os.environ", ENV), \
             patch("release_uploads.urlopen", return_value=response([draft])), \
             patch("release_uploads.subprocess.run") as gh:
            publish_github(TAG, self.npm, self.crate, self.sums)
        commands = [call.args[0] for call in gh.call_args_list]
        self.assertEqual([command[2] for command in commands], ["upload", "edit"])
        self.assertEqual(commands[0][-2:], [str(self.npm), str(self.sums)])
        self.assertNotIn(str(self.crate), commands[0])
        self.assertEqual(commands[1][-5:], ["--draft=false", "--title", TAG, "--notes", "Candidate"])

    def test_github_skips_a_complete_matching_release(self) -> None:
        sums_sha = "19a05c5d142cc56b07cac500240109127bd26f0618994eba0c91a8c0e8bc1935"
        assets = [
            {"name": path.name, "digest": f"sha256:{digest}"}
            for path, digest in [(self.crate, SHA256), (self.npm, SHA256), (self.sums, sums_sha)]
        ]
        release = {
            "tag_name": TAG, "draft": False, "prerelease": True, "assets": assets,
            "name": TAG, "body": "Candidate",
        }
        with patch.dict("os.environ", ENV), \
             patch("release_uploads.urlopen", return_value=response([release])), \
             patch("release_uploads.subprocess.run") as gh:
            publish_github(TAG, self.npm, self.crate, self.sums)
            gh.assert_not_called()

    def test_github_refuses_to_change_a_release_with_mismatched_digests(self) -> None:
        asset = {"name": self.crate.name, "digest": "sha256:wrong"}
        release = {
            "tag_name": TAG, "draft": False, "prerelease": True, "assets": [asset],
            "name": TAG, "body": "Candidate",
        }
        with patch.dict("os.environ", ENV), \
             patch("release_uploads.urlopen", return_value=response([release])), \
             patch("release_uploads.subprocess.run") as gh:
            with self.assertRaisesRegex(ValueError, "candidate.crate.*digest"):
                publish_github(TAG, self.npm, self.crate, self.sums)
            gh.assert_not_called()

    def test_github_refuses_to_change_a_published_release_with_mismatched_metadata(self) -> None:
        release = {
            "tag_name": TAG, "draft": False, "prerelease": True, "assets": [],
            "name": TAG, "body": "Candidate",
        }
        for field in ("body", "name"):
            with self.subTest(field=field), patch.dict("os.environ", ENV), \
                 patch("release_uploads.urlopen", return_value=response([release | {field: "Edited"}])), \
                 patch("release_uploads.subprocess.run") as gh:
                with self.assertRaisesRegex(ValueError, "title or notes"):
                    publish_github(TAG, self.npm, self.crate, self.sums)
                gh.assert_not_called()

    def test_github_does_not_upload_on_a_lookup_error(self) -> None:
        with patch.dict("os.environ", ENV), \
             patch("release_uploads.urlopen", side_effect=self.http_error(503)), \
             patch("release_uploads.subprocess.run") as gh:
            with self.assertRaises(HTTPError):
                publish_github(TAG, self.npm, self.crate, self.sums)
            gh.assert_not_called()
