#!/usr/bin/env python3
"""Compare a live Jenkins controller with the checked-in reference environment lock."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock-file", type=Path, required=True)
    parser.add_argument("--java-version-file", type=Path, required=True)
    parser.add_argument("--plugin-inventory", type=Path, required=True)
    parser.add_argument("--jenkins-headers", type=Path, required=True)
    parser.add_argument("--image-output", type=Path)
    args = parser.parse_args()

    lock = json.loads(args.lock_file.read_text(encoding="utf-8"))
    java_output = args.java_version_file.read_text(encoding="utf-8")
    headers = args.jenkins_headers.read_text(encoding="utf-8")
    inventory = json.loads(args.plugin_inventory.read_text(encoding="utf-8"))

    core_matches = re.findall(r"^X-Jenkins:\s*(\S+)", headers, flags=re.IGNORECASE | re.MULTILINE)
    assert core_matches and core_matches[-1] == lock["jenkinsCoreVersion"], {
        "expected": lock["jenkinsCoreVersion"],
        "observed": core_matches,
    }
    java_version = re.search(r"^\s*java.version\s*=\s*(\S+)", java_output, re.MULTILINE)
    java_vendor = re.search(r"^\s*java.vendor\s*=\s*(.+)$", java_output, re.MULTILINE)
    java_runtime = re.search(r"^\s*java.runtime.version\s*=\s*(\S+)", java_output, re.MULTILINE)
    assert java_version and java_version.group(1) == lock["javaVersion"], java_output
    assert java_vendor and java_vendor.group(1).strip() == lock["javaVendor"], java_output
    assert java_runtime and java_runtime.group(1) == lock["javaDistribution"].split()[-1], java_output
    assert "Temurin" in lock["javaDistribution"], lock["javaDistribution"]

    expected = dict(lock["basePlugins"])
    expected.update({item["name"]: item["version"] for item in lock["plugins"]})
    observed_items = inventory.get("plugins", [])
    observed = {item["shortName"]: item["version"] for item in observed_items}
    assert len(observed) == len(observed_items), "Jenkins returned duplicate plugin short names"
    assert observed == expected, {
        "missing": sorted(expected.items() - observed.items()),
        "unexpected": sorted(observed.items() - expected.items()),
    }

    if args.image_output:
        args.image_output.parent.mkdir(parents=True, exist_ok=True)
        args.image_output.write_text(lock["baseImage"] + "\n", encoding="utf-8")

    print(json.dumps({
        "environmentId": lock["environmentId"],
        "jenkinsCoreVersion": core_matches[-1],
        "javaVersion": lock["javaVersion"],
        "javaVendor": lock["javaVendor"],
        "javaDistribution": lock["javaDistribution"],
        "pluginCount": len(observed),
        "baseImage": lock["baseImage"],
    }, indent=2))


if __name__ == "__main__":
    main()
