"""Stage built Cargo targets in a new directory ready for pnpm pack."""

import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "aarch64-apple-darwin": ("darwin", "arm64"),
    "x86_64-apple-darwin": ("darwin", "x64"),
    "aarch64-unknown-linux-gnu": ("linux", "arm64"),
    "x86_64-unknown-linux-gnu": ("linux", "x64"),
}


def stage(target_dir: Path, package: Path, smoke_target: str | None) -> None:
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--locked", "--offline", "--format-version=1"],
        cwd=ROOT,
    ))
    crate, = metadata["packages"]
    package.mkdir()
    targets = {smoke_target: TARGETS[smoke_target]} if smoke_target else TARGETS
    for triple, (os_name, cpu) in targets.items():
        binary = package / "native" / f"{os_name}-{cpu}" / "blackhole"
        binary.parent.mkdir(parents=True)
        shutil.copyfile(target_dir / triple / "release" / "blackhole", binary)
        binary.chmod(0o755)
    launcher = package / "blackhole"
    shutil.copyfile(ROOT / "npm" / launcher.name, launcher)
    launcher.chmod(0o755)
    for name in ["README.md", "LICENSE", "THIRD_PARTY_NOTICES.md"]:
        shutil.copyfile(ROOT / name, package / name)
    manifest = {
        "name": "@gaspardip/blackhole",
        "version": crate["version"],
        "description": crate["description"],
        "license": crate["license"],
        "repository": crate["repository"],
        "bin": {"blackhole": launcher.name},
        "engines": {"node": ">=24"},
        "os": sorted({os_name for os_name, _ in targets.values()}),
        "cpu": sorted({cpu for _, cpu in targets.values()}),
    }
    if smoke_target:
        manifest["private"] = True
    (package / "package.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target_dir", type=Path)
    parser.add_argument("package_dir", type=Path)
    parser.add_argument("--smoke-target", choices=TARGETS, help="stage a private, single-target package")
    args = parser.parse_args()

    if args.package_dir.exists():
        parser.exit(1, f"destination already exists: {args.package_dir}\n")
    try:
        with tempfile.TemporaryDirectory(dir=args.package_dir.parent) as temp:
            package = Path(temp) / "package"
            stage(args.target_dir, package, args.smoke_target)
            package.rename(args.package_dir)
    except OSError as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
