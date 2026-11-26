#!/usr/bin/env python3
"""Assert the live Jenkins 2.356 Builder integration matrix and save sanitized evidence."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(sys.argv[1])


def load(package: str, filename: str) -> dict:
    return json.loads((ROOT / package / filename).read_text(encoding="utf-8"))


def outcome(manifest: dict, surface: str, name: str | None = None) -> dict:
    matches = [
        item
        for item in manifest["collection"]["sources"]
        if item["surface"] == surface and (name is None or item["name"] == name)
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"expected one outcome for surface={surface!r}, name={name!r}; got {matches!r}"
        )
    return matches[0]


def assert_controller(package: str, profile: str) -> tuple[dict, dict]:
    catalog = load(package, "catalog.json")
    manifest = load(package, "manifest.json")
    controller = catalog["controller"]
    assert controller["coreVersion"] == "2.356", controller.get("coreVersion")
    assert controller["collection"]["profile"] == profile
    return catalog, manifest


def assert_optional_success(manifest: dict, surface: str, name: str | None = None) -> None:
    status = outcome(manifest, surface, name)["status"]
    assert status in {"succeeded", "succeeded-with-truncation", "partial"}, (
        surface,
        name,
        status,
    )


quick, quick_manifest = assert_controller("quick", "quick")
assert quick["controller"]["collection"]["javaVersionStatus"] == "not-collected-by-profile"
assert quick["controller"]["collection"]["javaVendorStatus"] == "not-collected-by-profile"
assert outcome(quick_manifest, "script-console")["status"] == "skipped-by-profile"
assert outcome(quick_manifest, "library-source", "gsa-examples")["status"] == "skipped-by-profile"
assert outcome(quick_manifest, "pipeline-gdsl")["status"] == "skipped-by-profile"

standard, standard_manifest = assert_controller("standard", "standard")
assert standard["controller"]["collection"]["javaVersionStatus"] == "not-collected-by-profile"
assert standard["controller"]["collection"]["javaVendorStatus"] == "not-collected-by-profile"
assert outcome(standard_manifest, "script-console")["status"] == "skipped-by-profile"
assert_optional_success(standard_manifest, "pipeline-gdsl")
assert outcome(standard_manifest, "library-source", "gsa-examples")["status"] == "succeeded"
assert outcome(standard_manifest, "library-source", "gsa-examples")["revision"] == "7dfe2ab4bf7123b6aea29b0ec92caa2277cea5de"
assert outcome(standard_manifest, "library-source", "missing-commit")["status"] == "failed"

deep, deep_manifest = assert_controller("deep", "deep")
runtime = deep["controller"]["runtime"]
assert deep["controller"]["collection"]["javaVersionStatus"] == "available"
assert deep["controller"]["collection"]["javaVendorStatus"] == "available"
assert runtime["javaVersion"] == "17.0.3", runtime.get("javaVersion")
assert runtime["javaVendor"] == "Eclipse Adoptium", runtime.get("javaVendor")
assert runtime["apiTypeInventoryStatus"] in {"available", "partial"}
assert_optional_success(deep_manifest, "script-console")
assert_optional_success(deep_manifest, "pipeline-gdsl")
assert outcome(deep_manifest, "library-source", "gsa-examples")["status"] == "succeeded"
assert outcome(deep_manifest, "library-source", "gsa-examples")["revision"] == "7dfe2ab4bf7123b6aea29b0ec92caa2277cea5de"
assert outcome(deep_manifest, "library-source", "missing-commit")["status"] == "failed"

required_plugins = {
    "matrix-auth",
    "workflow-api",
    "workflow-basic-steps",
    "workflow-cps",
    "workflow-job",
    "workflow-step-api",
    "pipeline-model-definition",
    "script-security",
}
plugins = deep["controller"]["plugins"]
plugin_by_name = {plugin["shortName"]: plugin for plugin in plugins}
missing = sorted(required_plugins - plugin_by_name.keys())
assert not missing, f"missing required Jenkins plugins: {missing}"
assert plugin_by_name["matrix-auth"]["version"] == "2.6.11"
assert plugin_by_name["workflow-aggregator"]["version"] == "590.v6a_d052e5a_a_b_5"
assert plugin_by_name["pipeline-model-definition"]["version"] == "2.2114.v2654ca_721309"
assert plugin_by_name["workflow-cps"]["version"] == "2759.2761.vd6e8d2a_15980"

lock = json.loads((ROOT / "plugins.lock.json").read_text(encoding="utf-8"))
assert (ROOT / "image.txt").read_text(encoding="utf-8").strip() == lock["baseImage"]
expected_plugin_versions = dict(lock["basePlugins"])
expected_plugin_versions.update(
    {plugin["name"]: plugin["version"] for plugin in lock["plugins"]}
)
builder_plugin_versions = {
    plugin["shortName"]: plugin["version"] for plugin in plugins
}
assert builder_plugin_versions == expected_plugin_versions

api_inventory = json.loads((ROOT / "plugin-inventory-api.json").read_text(encoding="utf-8"))
api_plugin_versions = {
    plugin["shortName"]: plugin["version"] for plugin in api_inventory["plugins"]
}
assert builder_plugin_versions == api_plugin_versions
(ROOT / "plugin-inventory.json").write_text(
    json.dumps(plugins, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)

deep_readonly, deep_readonly_manifest = assert_controller("deep-readonly", "deep")
assert deep_readonly["controller"]["collection"]["javaVersionStatus"] == "unavailable"
assert deep_readonly["controller"]["collection"]["javaVendorStatus"] == "unavailable"
assert outcome(deep_readonly_manifest, "script-console")["status"] == "permission-denied"
assert_optional_success(deep_readonly_manifest, "pipeline-gdsl")
assert deep_readonly_manifest["collection"]["status"] == "partial"

summary = {
    "jenkinsCoreVersion": deep["controller"]["coreVersion"],
    "javaVersion": runtime["javaVersion"],
    "javaVendor": runtime["javaVendor"],
    "plugins": [
        {"shortName": plugin["shortName"], "version": plugin["version"]}
        for plugin in sorted(plugins, key=lambda item: item["shortName"])
    ],
    "profiles": {
        "quick": quick_manifest["collection"]["status"],
        "standard": standard_manifest["collection"]["status"],
        "deep-admin": deep_manifest["collection"]["status"],
        "deep-readonly": deep_readonly_manifest["collection"]["status"],
    },
}
(ROOT / "environment-summary.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
print(json.dumps(summary, indent=2, sort_keys=True))
