#!/usr/bin/env python3
"""Regression tests for plugin ordering and reference-environment drift."""

from __future__ import annotations

import importlib.util
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TEST_DIR = ROOT / "tests" / "jenkins-2.356"
SPEC = importlib.util.spec_from_file_location(
    "resolve_plugins", Path(__file__).with_name("resolve_plugins.py")
)
assert SPEC is not None and SPEC.loader is not None
RESOLVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RESOLVER)


def profile_library(path: Path) -> dict[str, str]:
    text = path.read_text(encoding="utf-8").split("libraries:", 1)[1]
    result = {}
    for key in ("name", "url", "revision", "commit"):
        match = re.search(r"^\s+" + key + r":\s*(\S+)\s*$", text, re.MULTILINE)
        if not match:
            raise AssertionError(f"{path} is missing library field {key}")
        result[key] = match.group(1)
    return result


class VersionOrderingTests(unittest.TestCase):
    def test_numeric_segment_is_newer_than_qualifier(self) -> None:
        newer = "1087.1089.v2f1b_9a_b_040e4"
        older = "1087.v16065d268466"
        self.assertTrue(RESOLVER.at_least(newer, older))
        self.assertFalse(RESOLVER.at_least(older, newer))

    def test_numeric_segments_compare_numerically(self) -> None:
        self.assertTrue(RESOLVER.at_least("2.356", "2.319.1"))
        self.assertFalse(RESOLVER.at_least("2.99", "2.100"))


class ReferenceEnvironmentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.lock = json.loads((TEST_DIR / "plugins.lock.json").read_text(encoding="utf-8"))

    def test_lock_pins_core_java_platform_and_base_image(self) -> None:
        lock = self.lock
        self.assertEqual(lock["environmentId"], "jenkins-2356")
        self.assertEqual(lock["jenkinsCoreVersion"], "2.356")
        self.assertEqual(lock["javaVersion"], "17.0.3")
        self.assertEqual(lock["javaVendor"], "Eclipse Adoptium")
        self.assertEqual(lock["javaDistribution"], "Eclipse Temurin 17.0.3+7")
        self.assertEqual(lock["platform"], "linux/amd64")
        self.assertRegex(lock["baseImage"], r"^jenkins/jenkins:2\.356-jdk17@sha256:[0-9a-f]{64}$")
        self.assertEqual(lock["hostPort"], 18080)
        self.assertTrue(lock["basePlugins"])
        self.assertTrue(lock["plugins"])

    def test_every_plugin_archive_is_version_and_checksum_pinned(self) -> None:
        plugins = self.lock["plugins"]
        names = [plugin["name"] for plugin in plugins]
        self.assertEqual(len(names), len(set(names)))
        for plugin in plugins:
            self.assertTrue(plugin["version"])
            self.assertTrue(plugin["requiredCore"])
            self.assertRegex(plugin["sha256"], r"^[0-9a-f]{64}$")
            self.assertEqual(
                plugin["url"],
                f"https://updates.jenkins.io/download/plugins/{plugin['name']}/{plugin['version']}/{plugin['name']}.hpi",
            )

    def test_dockerfile_and_compose_use_the_locked_image_platform(self) -> None:
        dockerfile = (TEST_DIR / "Dockerfile").read_text(encoding="utf-8")
        expected_from = f"FROM --platform={self.lock['platform']} {self.lock['baseImage']}"
        self.assertIn(expected_from, dockerfile.splitlines())

        compose = (ROOT / "demo" / "compose.yml").read_text(encoding="utf-8")
        self.assertIn(f"platform: {self.lock['platform']}", compose)
        self.assertIn("../target/jenkins-2.356-demo/docker-context", compose)

    def test_prebuilt_and_demo_profiles_share_the_locked_library_revision(self) -> None:
        expected = self.lock["sharedLibrary"]
        prebuilt = profile_library(ROOT / "profiles" / "prebuilt-jenkins-2.356.yml")
        demo = profile_library(ROOT / "demo" / "profile.yml")
        self.assertEqual(prebuilt, expected)
        self.assertEqual(demo, expected)
        self.assertIn("collectionProfile: deep", (ROOT / "profiles" / "prebuilt-jenkins-2.356.yml").read_text())
        self.assertIn("collectionProfile: standard", (ROOT / "demo" / "profile.yml").read_text())

    def test_local_demo_and_ci_materialize_the_same_lock_and_dockerfile(self) -> None:
        run = (TEST_DIR / "run.sh").read_text(encoding="utf-8")
        demo_up = (ROOT / "demo" / "up.sh").read_text(encoding="utf-8")
        prebuilt = (TEST_DIR / "build-prebuilt.sh").read_text(encoding="utf-8")

        self.assertIn("--lock-file tests/jenkins-2.356/plugins.lock.json", run)
        self.assertIn('--lock-file "$ROOT/tests/jenkins-2.356/plugins.lock.json"', demo_up)
        self.assertIn("--lock-file tests/jenkins-2.356/plugins.lock.json", prebuilt)
        self.assertIn('cp tests/jenkins-2.356/Dockerfile', run)
        self.assertIn('cp "$ROOT/tests/jenkins-2.356/Dockerfile"', demo_up)
        self.assertIn('cp tests/jenkins-2.356/Dockerfile', prebuilt)

        self.assertIn("python3 tests/jenkins-2.356/test_resolve_plugins.py", run)
        self.assertIn('python3 "$ROOT/tests/jenkins-2.356/test_resolve_plugins.py"', demo_up)
        self.assertIn("python3 tests/jenkins-2.356/test_resolve_plugins.py", prebuilt)

    def test_live_environment_checks_compare_java_and_plugin_inventory_to_lock(self) -> None:
        run = (TEST_DIR / "run.sh").read_text(encoding="utf-8")
        demo_up = (ROOT / "demo" / "up.sh").read_text(encoding="utf-8")
        prebuilt = (TEST_DIR / "build-prebuilt.sh").read_text(encoding="utf-8")
        self.assertIn("assert_reference_environment.py", run)
        self.assertIn("assert_reference_environment.py", demo_up)
        self.assertIn('runtime["javaVersion"] == lock["javaVersion"]', prebuilt)
        self.assertIn('runtime["javaVendor"] == lock["javaVendor"]', prebuilt)

    def test_documented_java_and_plugin_inventory_match_lock(self) -> None:
        readme = (TEST_DIR / "README.md").read_text(encoding="utf-8")
        self.assertIn(self.lock["javaDistribution"], readme)
        self.assertIn("plugins.lock.json", readme)
        self.assertIn("49 unique", readme)


if __name__ == "__main__":
    unittest.main()
