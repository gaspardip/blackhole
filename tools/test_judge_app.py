import base64
import json
import subprocess
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import judge_app


REPO = "/repos/gaspardip/blackhole"
BASE = "a" * 40
HEAD = "b" * 40


class JudgeApp(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.private_key = subprocess.check_output(
            ["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"],
            stderr=subprocess.DEVNULL, text=True,
        )

    def setUp(self) -> None:
        self.requests: list[tuple[str, str, str, dict | None]] = []
        self.pull = {
            "number": 7,
            "state": "open",
            "head": {"sha": HEAD},
            "base": {"ref": "main", "sha": BASE},
        }
        fixture = self

        class GitHub(BaseHTTPRequestHandler):
            def do_GET(self) -> None:
                replies = {f"{REPO}/pulls/7": fixture.pull, f"{REPO}/installation": {"id": 42}}
                fixture.requests.append(("GET", self.path, self.headers["Authorization"], None))
                self.reply(200, replies[self.path])

            def do_POST(self) -> None:
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                fixture.requests.append(("POST", self.path, self.headers["Authorization"], body))
                if self.path == "/app/installations/42/access_tokens":
                    self.reply(201, {"token": "installation-token"})
                else:
                    self.reply(201, {**body, "id": 93, "app": {"id": 19}})

            def reply(self, status: int, body: dict) -> None:
                self.send_response(status)
                self.end_headers()
                self.wfile.write(json.dumps(body).encode())

            def log_message(self, format: str, *args: object) -> None:
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), GitHub)
        self.api_url = f"http://127.0.0.1:{server.server_port}"
        thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(thread.join)
        self.addCleanup(server.shutdown)

    def publish(self, result: str, conclusion: str, summary: str, *, head: str = HEAD,
                base: str = BASE) -> int:
        return judge_app.publish(
            judge_app.Snapshot(7, HEAD, BASE),
            judge_app.Verdict(result, head, base, conclusion, summary),
            app_id="19", private_key=self.private_key, read_token="read-token", api_url=self.api_url,
        )

    def test_publishes_success_with_a_signed_jwt_and_repository_scoped_token(self) -> None:
        before = int(time.time())
        self.assertEqual(self.publish("success", "success", "blackhole ci passed."), 93)
        after = int(time.time())
        pull, installation, mint, check = self.requests
        self.assertEqual(pull, ("GET", f"{REPO}/pulls/7", "Bearer read-token", None))
        self.assertEqual(installation[:2], ("GET", f"{REPO}/installation"))
        self.assertEqual(mint, (
            "POST", "/app/installations/42/access_tokens", installation[2],
            {"repositories": ["blackhole"], "permissions": {"checks": "write"}},
        ))
        self.assertEqual(check, ("POST", f"{REPO}/check-runs", "Bearer installation-token", {
            "name": "blackhole judge", "head_sha": HEAD, "status": "completed",
            "conclusion": "success", "external_id": "pr/7",
            "output": {"title": "blackhole ci passed", "summary": "blackhole ci passed."},
        }))

        scheme, jwt = installation[2].split()
        self.assertEqual(scheme, "Bearer")
        header, claims, signature = jwt.split(".")
        self.assertEqual(json.loads(base64.urlsafe_b64decode(header + "==")), {"alg": "RS256", "typ": "JWT"})
        payload = json.loads(base64.urlsafe_b64decode(claims + "=="))
        self.assertEqual(payload["iss"], "19")
        self.assertLessEqual(before - 60, payload["iat"])
        self.assertLessEqual(payload["iat"], after)
        self.assertLess(before, payload["exp"])
        self.assertLessEqual(payload["exp"], after + 600)

        with tempfile.TemporaryDirectory() as folder:
            public_key = Path(folder) / "public.pem"
            public_key.write_bytes(subprocess.check_output(
                ["openssl", "pkey", "-pubout"], input=self.private_key.encode(),
            ))
            signed = Path(folder) / "signature"
            signed.write_bytes(base64.urlsafe_b64decode(signature + "=="))
            subprocess.run(
                ["openssl", "dgst", "-sha256", "-verify", public_key, "-signature", signed],
                input=f"{header}.{claims}".encode(), capture_output=True, check=True,
            )

    def test_publishes_blackhole_failure_as_a_completed_check(self) -> None:
        summary = "blackhole ci: red (protected)"
        self.publish("success", "failure", summary)
        self.assertEqual(self.requests[-1], ("POST", f"{REPO}/check-runs", "Bearer installation-token", {
            "name": "blackhole judge", "head_sha": HEAD, "status": "completed",
            "conclusion": "failure", "external_id": "pr/7",
            "output": {"title": "blackhole ci failed", "summary": summary},
        }))

    def test_posts_nothing_if_the_pr_head_or_target_changed(self) -> None:
        original = self.pull
        for change in ({"head": {"sha": "c" * 40}}, {"base": {"ref": "other", "sha": BASE}}):
            with self.subTest(change=change):
                self.requests.clear()
                self.pull = {**original, **change}
                with self.assertRaisesRegex(ValueError, "PR no longer matches"):
                    self.publish("success", "success", "blackhole ci passed.")
                self.assertEqual(self.requests, [("GET", f"{REPO}/pulls/7", "Bearer read-token", None)])

    def test_publishes_failure_when_judge_failed_or_was_cancelled(self) -> None:
        for result in ("failure", "cancelled"):
            with self.subTest(result=result):
                self.publish(result, "", "", head="", base="")
                body = self.requests[-1][3]
                self.assertEqual((body["head_sha"], body["status"], body["conclusion"]),
                                 (HEAD, "completed", "failure"))
                self.assertIn(result, body["output"]["summary"])

    def test_publishes_failure_for_unbound_or_invalid_judge_outputs(self) -> None:
        cases = (
            ("success", "blackhole ci passed.", "c" * 40, BASE),
            ("success", "blackhole ci passed.", HEAD, "c" * 40),
            ("neutral", "blackhole ci passed.", HEAD, BASE),
            ("success", "", HEAD, BASE),
        )
        for conclusion, summary, head, base in cases:
            with self.subTest(conclusion=conclusion, summary=summary, head=head, base=base):
                self.publish("success", conclusion, summary, head=head, base=base)
                body = self.requests[-1][3]
                self.assertEqual((body["head_sha"], body["conclusion"]), (HEAD, "failure"))
                self.assertIn("invalid verdict", body["output"]["summary"])

    def test_publishes_failure_when_main_advanced_during_the_run(self) -> None:
        self.pull["base"]["sha"] = "c" * 40
        self.publish("success", "success", "blackhole ci passed.")
        body = self.requests[-1][3]
        self.assertEqual((body["head_sha"], body["conclusion"]), (HEAD, "failure"))
        self.assertIn("main changed", body["output"]["summary"])
