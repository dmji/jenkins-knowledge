#!/usr/bin/env bash
set -euo pipefail

TAG="\${1:?Usage: package-prebuilt-release.sh vX.Y.Z PACKAGE_DIR OUTPUT_DIR COMMIT}"
PACKAGE_DIR="\${2:?Package directory is required}"
OUTPUT_DIR="\${3:?Output directory is required}"
COMMIT="\${4:?Source commit is required}"

if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
    echo "Release tag must be a version tag such as v1.2.3 or v1.2.3-rc.1" >&2
    exit 1
fi

ROOT="$(cd "$(dirname "\${BASH_SOURCE[0]}")/../.." && pwd)"
PACKAGE_DIR="$(cd "$PACKAGE_DIR" && pwd)"
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
ASSET_BASE="jenkins-knowledge-\${TAG}-jenkins-2.356-java17-deep"
ARCHIVE="$OUTPUT_DIR/$ASSET_BASE.tar.gz"
CHECKSUM="$ARCHIVE.sha256"
NOTES="$OUTPUT_DIR/RELEASE-NOTES.md"
TEMP="$(mktemp -d)"
trap 'rm -rf "$TEMP"' EXIT

rm -f "$ARCHIVE" "$CHECKSUM" "$NOTES"

python3 - "$PACKAGE_DIR" "$ROOT/tests/jenkins-2.356/plugins.lock.json" "$NOTES" "$TAG" "$COMMIT" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

package = Path(sys.argv[1])
lock = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
notes_path = Path(sys.argv[3])
tag, commit = sys.argv[4:6]

manifest = json.loads((package / "manifest.json").read_text(encoding="utf-8"))
catalog = json.loads((package / "catalog.json").read_text(encoding="utf-8"))
assert manifest["packageVersion"] == "1.0.0", manifest["packageVersion"]
assert manifest["catalogSchemaVersion"] == "1.0.0", manifest["catalogSchemaVersion"]
assert manifest["profile"] == "jenkins-2356-prebuilt", manifest["profile"]
assert manifest["collection"]["profile"] == "deep"
assert manifest["collection"]["status"] in {"complete", "partial"}

outcomes = manifest["collection"]["sources"]
script_console = [item for item in outcomes if item["surface"] == "script-console"]
assert len(script_console) == 1, script_console
assert script_console[0]["status"] == "succeeded" or (
    script_console[0]["status"] == "partial"
    and script_console[0].get("errorCode") == "enumeration-failed"
), script_console
unexpected = [
    item for item in outcomes
    if item["status"] != "succeeded"
    and not (item["surface"] == "script-console" and item["status"] == "partial"
             and item.get("errorCode") == "enumeration-failed")
]
assert not unexpected, unexpected
assert (manifest["collection"]["status"] == "partial") == (script_console[0]["status"] == "partial")

controller = catalog["controller"]
assert controller["coreVersion"] == lock["jenkinsCoreVersion"], controller["coreVersion"]
runtime = controller["runtime"]
assert runtime["javaVersion"].startswith("17."), runtime.get("javaVersion")
assert runtime["javaVendor"], "JVM vendor was not collected"

expected = dict(lock["basePlugins"])
expected.update({item["name"]: item["version"] for item in lock["plugins"]})
actual = {item["shortName"]: item["version"] for item in controller["plugins"]}
assert actual == expected, {"missing": sorted(expected.items() - actual.items()), "unexpected": sorted(actual.items() - expected.items())}

checksums = manifest["checksums"]
assert set(checksums) == {"catalog.json"}, checksums
for relative, digest in checksums.items():
    path = package / relative
    assert path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == digest, relative

files = sorted(path.relative_to(package).as_posix() for path in package.rglob("*") if path.is_file())
assert files == ["catalog.json", "manifest.json"], files
assert not any(path.is_symlink() for path in package.rglob("*")), "Package contains a symbolic link"

lines = [
    f"# Jenkins Knowledge {tag}",
    "",
    "## Reference environment",
    "",
    f"- Jenkins Core: **{controller['coreVersion']}**",
    f"- Java: **{runtime['javaVersion']}** ({runtime['javaVendor']})",
    f"- Collection profile: **{manifest['collection']['profile']}**",
    f"- Collection status: **{manifest['collection']['status']}**",
    f"- Knowledge Package format: **{manifest['packageVersion']}**",
    f"- Catalog schema: **{manifest['catalogSchemaVersion']}**",
    f"- Built at: {manifest['createdAt']}",
    f"- Repository commit: {commit}",
    f"- Source profile: {manifest['profile']}",
    f"- Installed plugins: **{len(actual)}**",
    "",
    "## Plugin inventory",
    "",
]
for name, version in sorted(actual.items()):
    lines.append(f"- {name} — {version}")
lines.extend(["", "## Collection notes", ""])
if script_console[0]["status"] == "partial":
    lines.append("- Jenkins 2.356 reports Script Console enumeration-failed for a subset of extension classes. The outcome remains marked partial in package metadata; all required controller, plugin inventory, Pipeline GDSL, and pinned Shared Library sources passed release validation.")
else:
    lines.append("- All configured collection sources completed successfully.")
lines.extend([
    "",
    "The archive contains the unchanged manifest.json and catalog.json. The manifest checksum covers the catalog; the separate .sha256 asset covers the complete compressed archive.",
    "",
    "## Download and verify",
    "",
    "Download both assets from this release, then run:",
    "",
    f"sha256sum -c {ASSET_BASE}.tar.gz.sha256",
    f"tar -xzf {ASSET_BASE}.tar.gz",
    "jenkins-runtime --package prebuilt-package plugins",
    "",
    "This package is an example for the exact reference environment above. It is not a compatibility guarantee for other Jenkins, Java, or plugin versions. For another environment, collect a custom package from that Jenkins instance.",
    "",
    "Runtime does not download, update, or contact Jenkins/Git when using this package; consumers select the extracted package path explicitly.",
    "",
])
notes_path.write_text("\n".join(lines), encoding="utf-8")
PY

tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    -cf - -C "$(dirname "$PACKAGE_DIR")" "$(basename "$PACKAGE_DIR")" | gzip -n > "$ARCHIVE"
sha256sum "$ARCHIVE" > "$CHECKSUM"
sha256sum --check "$CHECKSUM"

mkdir -p "$TEMP/extracted"
tar -xzf "$ARCHIVE" -C "$TEMP/extracted"
EXTRACTED="$TEMP/extracted/$(basename "$PACKAGE_DIR")"
test -f "$EXTRACTED/manifest.json"
test -f "$EXTRACTED/catalog.json"

target/debug/jenkins-runtime --package "$EXTRACTED" plugins > "$TEMP/plugins.json"
target/debug/jenkins-runtime --package "$EXTRACTED" steps > "$TEMP/steps.json"
target/debug/jenkins-runtime --package "$EXTRACTED" search example1 --source-kind shared-library > "$TEMP/library.json"
python3 - "$TEMP/plugins.json" "$TEMP/steps.json" "$TEMP/library.json" <<'PY'
import json
import sys
from pathlib import Path

plugins, steps, library = [json.loads(Path(value).read_text(encoding="utf-8")) for value in sys.argv[1:]]
assert any(item.get("shortName") == "workflow-basic-steps" for item in plugins.get("plugins", []))
names = {item.get("name") for item in steps.get("steps", [])}
assert names.intersection({"sh", "echo", "checkout"}), names
assert library.get("results"), "Extracted archive returned no Shared Library symbols"
PY

echo "Prepared and verified release assets:"
printf '  %s\n  %s\n  %s\n' "$ARCHIVE" "$CHECKSUM" "$NOTES"
