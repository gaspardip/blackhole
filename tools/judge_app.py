"""Publish a blackhole verdict from the repository's GitHub App."""

import base64
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from urllib.error import HTTPError
from urllib.request import Request, urlopen


REPO = "/repos/gaspardip/blackhole"


@dataclass(frozen=True)
class Snapshot:
    number: int
    head: str
    base: str


@dataclass(frozen=True)
class Verdict:
    job_result: str
    head: str
    base: str
    conclusion: str
    summary: str


def base64url(data: bytes) -> bytes:
    return base64.urlsafe_b64encode(data).rstrip(b"=")


def app_jwt(app_id: str, private_key: str) -> str:
    now = int(time.time())
    claims = json.dumps({"iss": app_id, "iat": now - 60, "exp": now + 540}).encode()
    message = base64url(b'{"alg":"RS256","typ":"JWT"}') + b"." + base64url(claims)
    with tempfile.NamedTemporaryFile() as pem:
        pem.write(private_key.encode())
        pem.flush()
        signature = subprocess.check_output(
            ["openssl", "dgst", "-sha256", "-sign", pem.name], input=message, stderr=subprocess.DEVNULL,
        )
    return (message + b"." + base64url(signature)).decode()


def github(api_url: str, path: str, token: str, body: dict | None = None) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    request = Request(api_url + path, data=data, headers={
        "Accept": "application/vnd.github+json",
        "Content-Type": "application/json",
        "User-Agent": "blackhole-judge",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    request.add_unredirected_header("Authorization", f"Bearer {token}")
    try:
        with urlopen(request, timeout=15) as response:
            if response.status == (201 if body is not None else 200):
                return json.load(response)
            raise RuntimeError(f"GitHub {request.get_method()} {path} returned HTTP {response.status}")
    except HTTPError as error:
        raise RuntimeError(f"GitHub {request.get_method()} {path} returned HTTP {error.code}") from error


def publish(snapshot: Snapshot, verdict: Verdict, *, app_id: str, private_key: str,
            read_token: str, api_url: str = "https://api.github.com") -> int:
    if not re.fullmatch(r"[1-9][0-9]*", app_id):
        raise ValueError("expected a numeric GitHub App ID")
    pull = github(api_url, f"{REPO}/pulls/{snapshot.number}", read_token)
    if (pull["state"] != "open" or pull["number"] != snapshot.number
            or pull["base"]["ref"] != "main" or pull["head"]["sha"] != snapshot.head):
        raise ValueError("PR no longer matches the event head and target branch")

    conclusion = "failure"
    if verdict.job_result != "success":
        summary = f"The judge job ended with {verdict.job_result}."
    elif pull["base"]["sha"] != snapshot.base:
        summary = "The tip of main changed while the judge ran. Update the branch and rerun the judge."
    elif ((verdict.head, verdict.base) != (snapshot.head, snapshot.base)
          or verdict.conclusion not in ("success", "failure") or not verdict.summary):
        summary = "The judge job returned an invalid verdict."
    else:
        conclusion = verdict.conclusion
        summary = verdict.summary

    jwt = app_jwt(app_id, private_key)
    installation = github(api_url, f"{REPO}/installation", jwt)["id"]
    token = github(api_url, f"/app/installations/{installation}/access_tokens", jwt, {
        "repositories": ["blackhole"], "permissions": {"checks": "write"},
    })["token"]
    body = {
        "name": "blackhole judge", "head_sha": snapshot.head, "external_id": f"pr/{snapshot.number}",
        "status": "completed", "conclusion": conclusion,
        "output": {
            "title": "blackhole ci passed" if conclusion == "success" else "blackhole ci failed",
            "summary": summary,
        },
    }
    check = github(api_url, f"{REPO}/check-runs", token, body)
    if check["app"]["id"] != int(app_id) or any(
        check[key] != body[key] for key in ("name", "head_sha", "status", "conclusion")
    ):
        raise ValueError("GitHub returned an unexpected check run")
    return check["id"]


def main() -> None:
    env = os.environ
    snapshot = Snapshot(int(env["PR_NUMBER"]), env["PR_HEAD_SHA"], env["PR_BASE_SHA"])
    verdict = Verdict(env["JUDGE_RESULT"], env["JUDGE_HEAD_SHA"], env["JUDGE_BASE_SHA"],
                      env["JUDGE_CONCLUSION"], env["JUDGE_SUMMARY"])
    run_id = publish(snapshot, verdict, app_id=env["JUDGE_APP_ID"],
                     private_key=env["JUDGE_APP_PRIVATE_KEY"], read_token=env["PR_READ_TOKEN"])
    print(f"Published blackhole judge check run {run_id}.")


if __name__ == "__main__":
    try:
        main()
    except (KeyError, OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(f"blackhole judge: {error}")
