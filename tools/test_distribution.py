import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARIES = {
    "aarch64-apple-darwin": "darwin-arm64",
    "x86_64-apple-darwin": "darwin-x64",
    "aarch64-unknown-linux-gnu": "linux-arm64",
    "x86_64-unknown-linux-gnu": "linux-x64",
}


class Distribution(unittest.TestCase):
    def setUp(self) -> None:
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.package = self.root / "package"

    def stage(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, ROOT / "tools/stage_npm.py", self.root, self.package, *args],
            check=check, capture_output=True, text=True,
        )

    def write_input(self, triple: str) -> None:
        binary = self.root / triple / "release" / "blackhole"
        binary.parent.mkdir(parents=True)
        binary.write_text(triple)

    def test_crate_contains_only_rust_sources_and_package_metadata(self) -> None:
        files = subprocess.check_output(
            ["cargo", "package", "--offline", "--locked", "--allow-dirty", "--list"],
            cwd=ROOT, text=True,
        ).splitlines()
        self.assertEqual(set(files), {
            ".cargo_vcs_info.json", "Cargo.toml", "Cargo.toml.orig", "Cargo.lock",
            "README.md", "LICENSE", "THIRD_PARTY_NOTICES.md",
            *(p.relative_to(ROOT).as_posix() for p in (ROOT / "src").rglob("*.rs")),
        })

    def test_release_stages_each_native_executable_and_notices(self) -> None:
        for triple in BINARIES:
            self.write_input(triple)
        self.stage()
        manifest = json.loads((self.package / "package.json").read_text())
        self.assertEqual(manifest["name"], "@gaspardip/blackhole")
        self.assertFalse(manifest.get("private"))
        self.assertTrue((self.package / manifest["bin"]["blackhole"]).is_file())
        for triple, platform in BINARIES.items():
            binary = self.package / "native" / platform / "blackhole"
            self.assertEqual(binary.read_text(), triple)
            self.assertTrue(binary.stat().st_mode & 0o111)
        for notice in ["LICENSE", "THIRD_PARTY_NOTICES.md"]:
            self.assertEqual((self.package / notice).read_bytes(), (ROOT / notice).read_bytes())

    def test_missing_input_names_the_binary_and_leaves_destination_available(self) -> None:
        missing = "x86_64-unknown-linux-gnu"
        for triple in BINARIES.keys() - {missing}:
            self.write_input(triple)
        result = self.stage(check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(missing, result.stderr)
        self.assertFalse(self.package.exists())

    def test_single_target_is_a_private_platform_specific_smoke_package(self) -> None:
        triple = "x86_64-unknown-linux-gnu"
        self.write_input(triple)
        self.stage("--smoke-target", triple)
        manifest = json.loads((self.package / "package.json").read_text())
        self.assertTrue(manifest["private"])
        self.assertEqual(manifest["os"], ["linux"])
        self.assertEqual(manifest["cpu"], ["x64"])
        self.assertEqual(list((self.package / "native").iterdir()), [self.package / "native/linux-x64"])
