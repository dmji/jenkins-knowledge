#!/usr/bin/env python3
"""Materialize and validate the digest-pinned Jenkins reference environment."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import zipfile
from pathlib import Path


def version_key(value: str) -> tuple[tuple[int, int | str], ...]:
    return tuple(
        (1, int(part)) if part.isdigit() else (0, part.lower())
        for part in re.findall(r"[0-9]+|[A-Za-z]+", value)
    )


def at_least(actual: str, required: str) -> bool:
    return version_key(actual) >= version_key(required)


def parse_manifest(archive_path: Path) -> dict:
    with zipfile.ZipFile(archive_path) as archive:
        raw = archive.read("META-INF/MANIFEST.MF").decode("utf-8")

    unfolded: list[str] = []
    for line in raw.replace("\r\n", "\n").split("\n"):
        if line.startswith(" ") and unfolded:
            unfolded[-1] += line[1:]
        elif line:
            unfolded.append(line)

    attributes = {}
    for line in unfolded:
        if ": " in line:
            key, value = line.split(": ", 1)
            attributes[key] = value

    dependencies = []
    for item in attributes.get("Plugin-Dependencies", "").split(","):
        parts = [part.strip() for part in item.split(";")]
        if not parts or not parts[0]:
            continue
        if any(part.lower() == "resolution:=optional" for part in parts[1:]):
            continue
        name, version = parts[0].split(":", 1)
        dependencies.append((name, version))

    return {
        "name": attributes.get("Short-Name"),
        "version": attributes.get("Plugin-Version"),
        "requiredCore": attributes.get("Jenkins-Version")
        or attributes.get("Hudson-Version", "0"),
        "minimumJava": attributes.get("Minimum-Java-Version", "1.8"),
        "dependencies": dependencies,
    }


def validate_environment_lock(lock: dict) -> None:
    required = (
        "environmentId",
        "platform",
        "jenkinsCoreVersion",
        "javaVersion",
        "javaVendor",
        "javaDistribution",
        "baseImage",
        "basePlugins",
        "sharedLibrary",
        "plugins",
    )
    missing = [key for key in required if key not in lock]
    if missing:
        raise RuntimeError(f"Reference environment lock is missing fields: {missing}")

    if not re.fullmatch(r"jenkins/jenkins:[^@]+@sha256:[0-9a-f]{64}", lock["baseImage"]):
        raise RuntimeError("Reference Jenkins base image must be pinned by digest")
    if lock["platform"] != "linux/amd64":
        raise RuntimeError(f"Unexpected reference platform: {lock['platform']}")
    if not lock["javaVersion"] or not lock["javaVendor"] or not lock["javaDistribution"]:
        raise RuntimeError("Reference Java version and vendor must be explicit")

    library = lock["sharedLibrary"]
    if not library.get("url", "").startswith("https://"):
        raise RuntimeError("Reference Shared Library must use HTTPS")
    if not re.fullmatch(r"[0-9a-f]{40}", library.get("commit", "")):
        raise RuntimeError("Reference Shared Library must be pinned to a full commit SHA")

    if not isinstance(lock["basePlugins"], dict) or not lock["basePlugins"]:
        raise RuntimeError("Reference base-image plugin inventory must be explicit")
    if not isinstance(lock["plugins"], list) or not lock["plugins"]:
        raise RuntimeError("Reference plugin lock must not be empty")


def load_locked_plugins(lock_file: Path, output_dir: Path) -> dict:
    lock = json.loads(lock_file.read_text(encoding="utf-8"))
    validate_environment_lock(lock)
    core_version = lock["jenkinsCoreVersion"]
    java_version = lock["javaVersion"]
    base_plugins = lock["basePlugins"]

    installed_versions = dict(base_plugins)
    seen = set()
    for plugin in lock["plugins"]:
        name = plugin["name"]
        version = plugin["version"]
        if name in seen:
            raise RuntimeError(f"Duplicate plugin entry in lock: {name}")
        seen.add(name)

        expected_url = f"https://updates.jenkins.io/download/plugins/{name}/{version}/{name}.hpi"
        if plugin["url"] != expected_url:
            raise RuntimeError(f"Unexpected download URL for locked plugin {name}")
        if not re.fullmatch(r"[0-9a-f]{64}", plugin["sha256"]):
            raise RuntimeError(f"Missing SHA-256 pin for locked plugin {name}")

        archive_path = output_dir / f"{name}.jpi"
        digest = (
            hashlib.sha256(archive_path.read_bytes()).hexdigest()
            if archive_path.exists()
            else ""
        )
        if digest != plugin["sha256"]:
            subprocess.run(
                [
                    "curl", "--fail", "--location", "--silent", "--show-error",
                    "--retry", "5", "--retry-connrefused", "--connect-timeout", "20",
                    "--max-time", "180", "--proto", "=https", "--tlsv1.2",
                    plugin["url"], "--output", str(archive_path),
                ],
                check=True,
            )
            digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
        if digest != plugin["sha256"]:
            raise RuntimeError(
                f"SHA-256 mismatch for locked plugin {name}:{version}: {digest}"
            )

        manifest = parse_manifest(archive_path)
        if manifest["name"] != name or manifest["version"] != version:
            raise RuntimeError(f"Unexpected locked plugin identity for {name}")
        if manifest["requiredCore"] != plugin["requiredCore"]:
            raise RuntimeError(f"Jenkins minimum changed for locked plugin {name}")
        if not at_least(core_version, manifest["requiredCore"]):
            raise RuntimeError(
                f"Locked plugin {name} requires Jenkins {manifest['requiredCore']}"
            )
        if not at_least(java_version, manifest["minimumJava"]):
            raise RuntimeError(
                f"Locked plugin {name} requires Java {manifest['minimumJava']}"
            )
        installed_versions[name] = version
        plugin["_resolvedDependencies"] = manifest["dependencies"]

    for plugin in lock["plugins"]:
        for dependency, required_version in plugin.pop("_resolvedDependencies"):
            actual_version = installed_versions.get(dependency)
            if actual_version is None or not at_least(actual_version, required_version):
                raise RuntimeError(
                    f"Plugin lock is incomplete: {plugin['name']} requires "
                    f"{dependency}>={required_version}, found {actual_version}"
                )

    return lock


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--lock-file", type=Path, required=True)
    parser.add_argument("--lock-output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)

    lock = load_locked_plugins(args.lock_file, args.output)
    args.lock_output.parent.mkdir(parents=True, exist_ok=True)
    args.lock_output.write_text(
        json.dumps(lock, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        f"Prepared {len(lock['plugins'])} locked plugin archives for "
        f"Jenkins {lock['jenkinsCoreVersion']} / Java {lock['javaVersion']} "
        f"({lock['environmentId']})"
    )


if __name__ == "__main__":
    main()
