#!/usr/bin/env python3
"""Resolve and checksum-lock the Jenkins 2.356 plugin dependency closure."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import zipfile
from collections import deque
from pathlib import Path

JENKINS_CORE = "2.356"
BASE_IMAGE = (
    "jenkins/jenkins:2.356-jdk17@sha256:"
    "211419c659bd04b2d065ead531aad35ea47471489dcf714ab2c8a30cb9870850"
)
BASE_PLUGINS = {
    "javax-activation-api": "1.2.0-2",
    "javax-mail-api": "1.6.2-5",
    "matrix-auth": "2.6.11",
    "sshd": "3.0.3",
}
ROOT_SHA256 = {
    ("workflow-aggregator", "590.v6a_d052e5a_a_b_5"):
        "3f4485827a8e045663c6474a39189051bae17657d6e5853bb4ea5ef396df1961",
    ("pipeline-model-definition", "2.2114.v2654ca_721309"):
        "4a6df22729aa8437c1f6cd2bfd96a88fefa2466bf106aa6877a3dd8783fec166",
    ("workflow-cps", "2759.2761.vd6e8d2a_15980"):
        "f28ed252e632c8c23926c960fd4af5469febef2e58e8a2f33923742d6f5d643c",
}


def version_key(value: str) -> tuple[tuple[int, int | str], ...]:
    return tuple(
        (0, int(part)) if part.isdigit() else (1, part.lower())
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


def download_plugin(name: str, version: str, output_dir: Path) -> dict:
    output_dir.mkdir(parents=True, exist_ok=True)
    archive_path = output_dir / f"{name}.jpi"
    url = f"https://updates.jenkins.io/download/plugins/{name}/{version}/{name}.hpi"
    subprocess.run(
        [
            "curl", "--fail", "--location", "--silent", "--show-error",
            "--retry", "5", "--retry-connrefused", "--connect-timeout", "20",
            "--max-time", "180", "--proto", "=https", "--tlsv1.2",
            url, "--output", str(archive_path),
        ],
        check=True,
    )
    digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    expected = ROOT_SHA256.get((name, version))
    if expected and digest != expected:
        raise RuntimeError(f"SHA-256 mismatch for {name}:{version}: {digest}")

    manifest = parse_manifest(archive_path)
    if manifest["name"] != name or manifest["version"] != version:
        raise RuntimeError(
            f"Unexpected archive identity for {name}:{version}: "
            f"{manifest['name']}:{manifest['version']}"
        )
    if not at_least(JENKINS_CORE, manifest["requiredCore"]):
        raise RuntimeError(
            f"{name}:{version} requires Jenkins {manifest['requiredCore']}, "
            f"target is {JENKINS_CORE}"
        )
    if not at_least("17", manifest["minimumJava"]):
        raise RuntimeError(
            f"{name}:{version} requires Java {manifest['minimumJava']}, target is 17"
        )

    return {
        "name": name,
        "version": version,
        "sha256": digest,
        "url": url,
        "requiredCore": manifest["requiredCore"],
        "dependencies": manifest["dependencies"],
    }


def read_roots(path: Path) -> list[tuple[str, str]]:
    roots = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            name, version = line.split(":", 1)
            roots.append((name.strip(), version.strip()))
    if not roots:
        raise RuntimeError(f"No root plugins in {path}")
    return roots


def resolve_plugins(roots: list[tuple[str, str]], output_dir: Path) -> list[dict]:
    resolved: dict[str, dict] = {}
    queue: deque[tuple[str, str]] = deque()

    # Install roots first so their pinned versions take precedence over
    # lower minimum versions declared by other roots.
    for name, version in roots:
        if name in BASE_PLUGINS and at_least(BASE_PLUGINS[name], version):
            continue
        plugin = download_plugin(name, version, output_dir)
        resolved[name] = plugin
        queue.extend(plugin["dependencies"])

    while queue:
        name, required_version = queue.popleft()
        if name in BASE_PLUGINS and at_least(BASE_PLUGINS[name], required_version):
            continue
        current = resolved.get(name)
        if current and at_least(current["version"], required_version):
            continue

        plugin = download_plugin(name, required_version, output_dir)
        resolved[name] = plugin
        queue.extend(plugin["dependencies"])

    return [resolved[name] for name in sorted(resolved)]


def load_locked_plugins(lock_file: Path, output_dir: Path) -> dict:
    lock = json.loads(lock_file.read_text(encoding="utf-8"))
    if lock["jenkinsCoreVersion"] != JENKINS_CORE or lock["baseImage"] != BASE_IMAGE:
        raise RuntimeError("Plugin lock targets a different Jenkins image")
    if lock["basePlugins"] != BASE_PLUGINS:
        raise RuntimeError("Base plugin inventory in lock does not match the pinned image")

    installed_versions = dict(BASE_PLUGINS)
    installed_versions.update(
        {plugin["name"]: plugin["version"] for plugin in lock["plugins"]}
    )
    manifests = []
    for plugin in lock["plugins"]:
        name = plugin["name"]
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
            raise RuntimeError(f"SHA-256 mismatch for locked plugin {name}:{plugin['version']}")
        root_digest = ROOT_SHA256.get((name, plugin["version"]))
        if root_digest and digest != root_digest:
            raise RuntimeError(f"Root plugin checksum changed for {name}:{plugin['version']}")
        manifest = parse_manifest(archive_path)
        if manifest["name"] != name or manifest["version"] != plugin["version"]:
            raise RuntimeError(f"Unexpected locked plugin identity for {name}")
        if not at_least(JENKINS_CORE, manifest["requiredCore"]):
            raise RuntimeError(f"Locked plugin {name} requires Jenkins {manifest['requiredCore']}")
        if not at_least("17", manifest["minimumJava"]):
            raise RuntimeError(f"Locked plugin {name} requires Java {manifest['minimumJava']}")
        manifests.append((name, manifest))

    for name, manifest in manifests:
        for dependency, required_version in manifest["dependencies"]:
            actual_version = installed_versions.get(dependency)
            if actual_version is None or not at_least(actual_version, required_version):
                raise RuntimeError(
                    f"Plugin lock is incomplete: {name} requires "
                    f"{dependency}>={required_version}, found {actual_version}"
                )
    return lock


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--roots-file", type=Path)
    parser.add_argument("--lock-file", type=Path)
    parser.add_argument("--lock-output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)

    if args.lock_file:
        lock = load_locked_plugins(args.lock_file, args.output)
    else:
        if not args.roots_file:
            parser.error("--roots-file is required when --lock-file is not supplied")
        plugins = resolve_plugins(read_roots(args.roots_file), args.output)
        lock = {
            "jenkinsCoreVersion": JENKINS_CORE,
            "baseImage": BASE_IMAGE,
            "basePlugins": BASE_PLUGINS,
            "plugins": [
                {key: value for key, value in plugin.items() if key != "dependencies"}
                for plugin in plugins
            ],
        }

    args.lock_output.parent.mkdir(parents=True, exist_ok=True)
    args.lock_output.write_text(
        json.dumps(lock, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        f"Prepared {len(lock['plugins'])} pinned Jenkins plugins "
        f"for Core {JENKINS_CORE}"
    )


if __name__ == "__main__":
    main()
