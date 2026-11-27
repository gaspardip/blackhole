"""Upload an existing .crate using Cargo's registry web API."""

import io
import json
import os
import sys
import tarfile
import tomllib
from pathlib import Path
from urllib.error import URLError
from urllib.request import Request, urlopen


def dependencies(tables: dict, target: str | None = None) -> list[dict]:
    result = []
    for table, kind in (
        ("dependencies", "normal"), ("dev-dependencies", "dev"), ("build-dependencies", "build")
    ):
        for name, specification in tables.get(table, {}).items():
            dep = {"version": specification} if isinstance(specification, str) else specification
            if "registry" in dep or "registry-index" in dep:
                raise ValueError("release only supports crates.io dependencies")
            original = dep.get("package", name)
            result.append({
                "name": original, "version_req": dep["version"], "features": dep.get("features", []),
                "optional": dep.get("optional", False),
                "default_features": dep.get("default-features", True),
                "target": target, "kind": kind, "registry": None,
                "explicit_name_in_toml": name if original != name else None,
            })
    return result


def publish(file: Path, token: str) -> None:
    if not token:
        raise ValueError("missing crates.io trusted publishing credential")
    archive_bytes = file.read_bytes()
    with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:gz") as archive:
        def read(name: str) -> str:
            member = archive.extractfile(f"{file.stem}/{name}")
            if member is None:
                raise ValueError("crate metadata entry is not a file")
            with member:
                return member.read().decode()

        manifest = tomllib.loads(read("Cargo.toml"))
        package = manifest["package"]
        if file.stem != f"{package['name']}-{package['version']}":
            raise ValueError("crate name and version do not match its archive filename")
        deps = dependencies(manifest)
        for target, tables in manifest.get("target", {}).items():
            deps.extend(dependencies(tables, target))
        readme = package.get("readme") or None
        metadata = {
            field: package.get(field)
            for field in ("description", "documentation", "homepage", "license", "repository", "links")
        }
        metadata.update({field: package.get(field, []) for field in ("authors", "keywords", "categories")})
        metadata.update(
            name=package["name"], vers=package["version"], deps=deps,
            features=manifest.get("features", {}), badges=manifest.get("badges", {}),
            license_file=package.get("license-file"), rust_version=package.get("rust-version"),
            readme_file=readme, readme=read(readme) if readme else None,
        )

    fields = (json.dumps(metadata).encode(), archive_bytes)
    # Cargo's publish API prefixes both the JSON and the archive with little-endian lengths.
    body = b"".join(len(field).to_bytes(4, "little") + field for field in fields)
    request = Request("https://crates.io/api/v1/crates/new", data=body, method="PUT", headers={
        "Authorization": token, "Content-Type": "application/octet-stream",
        "Accept": "application/json", "User-Agent": "blackhole-release",
    })
    with urlopen(request, timeout=120) as response:
        result = json.loads(response.read())
    if result.get("errors"):
        raise ValueError("crates.io rejected the archive")
    if any(result.get("warnings", {}).values()):
        print("crates.io returned publishing warnings", file=sys.stderr)


if __name__ == "__main__":
    try:
        publish(Path(sys.argv[1]), os.environ.get("CARGO_REGISTRY_TOKEN", ""))
    except (OSError, ValueError, tarfile.TarError, KeyError) as error:
        status = getattr(error, "code", None)
        detail = f"HTTP {status}" if isinstance(error, URLError) and status else type(error).__name__
        sys.exit(f"crate upload failed ({detail})")
