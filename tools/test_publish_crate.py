import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from publish_crate import publish


class PublishCrate(unittest.TestCase):
    def test_upload_uses_the_verified_archive_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            file = Path(tmp) / "blackhole-check-1.2.3-rc.1.crate"
            members = {
                "Cargo.toml": b"""
[package]
name = "blackhole-check"
version = "1.2.3-rc.1"
description = "Example"
license = "MIT"
readme = "README.md"
rust-version = "1.99"

[dependencies.parser]
package = "parser-core"
version = "1.2.3"
default-features = false
features = ["syntax"]

[dev-dependencies]
tempfile = "3"
""",
                "README.md": b"Example README\n",
            }
            with tarfile.open(file, "w:gz") as archive:
                for name, data in members.items():
                    member = tarfile.TarInfo(f"{file.stem}/{name}")
                    member.size = len(data)
                    archive.addfile(member, io.BytesIO(data))

            with patch("publish_crate.urlopen") as upload:
                upload.return_value.__enter__.return_value.read.return_value = b"{}"
                publish(file, "temporary-credential")
                request = upload.call_args.args[0]

            self.assertEqual(request.full_url, "https://crates.io/api/v1/crates/new")
            self.assertEqual(request.method, "PUT")
            self.assertEqual(request.get_header("Authorization"), "temporary-credential")
            self.assertEqual(request.get_header("Content-type"), "application/octet-stream")
            end = 4 + int.from_bytes(request.data[:4], "little")
            metadata = json.loads(request.data[4:end])
            self.assertEqual(metadata["name"], "blackhole-check")
            self.assertEqual(metadata["vers"], "1.2.3-rc.1")
            self.assertEqual(metadata["readme"], "Example README\n")
            self.assertEqual(metadata["rust_version"], "1.99")
            self.assertEqual(metadata["deps"], [
                {
                    "name": "parser-core", "version_req": "1.2.3", "features": ["syntax"],
                    "optional": False, "default_features": False, "target": None,
                    "kind": "normal", "registry": None, "explicit_name_in_toml": "parser",
                },
                {
                    "name": "tempfile", "version_req": "3", "features": [],
                    "optional": False, "default_features": True, "target": None,
                    "kind": "dev", "registry": None, "explicit_name_in_toml": None,
                },
            ])
            self.assertEqual(int.from_bytes(request.data[end:end + 4], "little"), file.stat().st_size)
            self.assertEqual(request.data[end + 4:], file.read_bytes())
