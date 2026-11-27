#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
OUT="$ROOT/target/prebuilt-package"
TEMP="$(mktemp -d)"
IMAGE="jenkins-knowledge-prebuilt-2356"
CONTAINER="jenkins-knowledge-prebuilt-${GITHUB_RUN_ID:-local}"
JENKINS_URL="http://127.0.0.1:18080"
TOKEN_FILE="$TEMP/tokens.env"
ENV_FILE="$TEMP/jenkins.env"

cleanup() {
    status=$?
    trap - EXIT
    if [ "$status" -ne 0 ]; then
        docker logs "$CONTAINER" 2>&1 | tail -n 120 || true
    fi
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    docker image rm "$IMAGE" >/dev/null 2>&1 || true
    rm -rf "$TEMP"
    exit "$status"
}
trap cleanup EXIT

rm -rf "$OUT"
mkdir -p "$(dirname "$OUT")" "$TEMP/docker-context/plugins"
cargo build --locked --manifest-path crates/knowledge-builder/Cargo.toml
cargo build --locked --manifest-path crates/jenkins-runtime/Cargo.toml --bin jenkins-runtime

python3 tests/jenkins-2.356/test_resolve_plugins.py
cp tests/jenkins-2.356/Dockerfile "$TEMP/docker-context/Dockerfile"
python3 tests/jenkins-2.356/resolve_plugins.py \
    --lock-file tests/jenkins-2.356/plugins.lock.json \
    --output "$TEMP/docker-context/plugins" \
    --lock-output "$TEMP/plugins.lock.json"

admin_password="$(openssl rand -hex 32)"
reader_password="$(openssl rand -hex 32)"
collector_password="$(openssl rand -hex 32)"
cat > "$ENV_FILE" <<EOF
JAVA_OPTS=-Djenkins.install.runSetupWizard=false
TEST_ADMIN_PASSWORD=$admin_password
TEST_READER_PASSWORD=$reader_password
TEST_COLLECTOR_PASSWORD=$collector_password
JENKINS_KNOWLEDGE_TOKEN_FILE=/tmp/jenkins-knowledge-tokens.env
EOF
chmod 600 "$ENV_FILE"

docker build --platform linux/amd64 --pull=false -t "$IMAGE" "$TEMP/docker-context"
docker run --detach \
    --name "$CONTAINER" \
    --publish 127.0.0.1:18080:8080 \
    --env-file "$ENV_FILE" \
    --volume "$ROOT/tests/jenkins-2.356/init.groovy:/usr/share/jenkins/ref/init.groovy.d/99-jenkins-knowledge.groovy:ro" \
    "$IMAGE" >/dev/null

for attempt in $(seq 1 120); do
    if docker cp "$CONTAINER:/tmp/jenkins-knowledge-tokens.env" "$TOKEN_FILE" >/dev/null 2>&1; then
        break
    fi
    if [ "$(docker inspect -f '{{.State.Running}}' "$CONTAINER" 2>/dev/null || true)" != "true" ]; then
        echo "Jenkins stopped before creating collection credentials" >&2
        exit 1
    fi
    sleep 5
done
test -s "$TOKEN_FILE"

admin_token="$(sed -n 's/^ADMIN_TOKEN=//p' "$TOKEN_FILE")"
collector_token="$(sed -n 's/^COLLECTOR_TOKEN=//p' "$TOKEN_FILE")"
test -n "$admin_token"
test -n "$collector_token"

ready=false
for attempt in $(seq 1 60); do
    if curl --silent --show-error --fail --user "admin:$admin_token" "$JENKINS_URL/api/json" >/dev/null; then
        ready=true
        break
    fi
    sleep 5
done
[ "$ready" = true ] || { echo "Jenkins API did not become ready" >&2; exit 1; }

# Prove the dedicated collector can run the fixed read-only probe without ADMINISTER.
probe="$(curl --silent --show-error --fail --user "collector:$collector_token" \
    --data-urlencode 'script=println("jenkins-knowledge-collector-ready")' \
    "$JENKINS_URL/scriptText")"
grep -q 'jenkins-knowledge-collector-ready' <<<"$probe"

echo "Collecting the deep reference package..."
JENKINS_USERNAME=collector \
JENKINS_API_TOKEN="$collector_token" \
JENKINS_KNOWLEDGE_CACHE_DIR="$TEMP/cache" \
    target/debug/knowledge-builder build profiles/prebuilt-jenkins-2.356.yml "$OUT"

echo "Validating package metadata, completeness, plugin lock, and Runtime queries..."
target/debug/jenkins-runtime --package "$OUT" search echo --plugin workflow-basic-steps > "$TEMP/plugin-query.json"
target/debug/jenkins-runtime --package "$OUT" search example1 --source-kind shared-library > "$TEMP/library-query.json"
target/debug/jenkins-runtime --package "$OUT" steps > "$TEMP/pipeline-query.json"

python3 - "$OUT" "$TEMP/plugins.lock.json" "$TEMP/plugin-query.json" "$TEMP/library-query.json" "$TEMP/pipeline-query.json" "$collector_token" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

package = Path(sys.argv[1])
lock_path = Path(sys.argv[2])
queries = [Path(value) for value in sys.argv[3:6]]
secret = sys.argv[6]

manifest = json.loads((package / "manifest.json").read_text(encoding="utf-8"))
catalog = json.loads((package / "catalog.json").read_text(encoding="utf-8"))
lock = json.loads(lock_path.read_text(encoding="utf-8"))

assert manifest["profile"] == "jenkins-2356-prebuilt", manifest["profile"]
assert manifest["collection"]["profile"] == "deep"
assert manifest["collection"]["status"] in {"complete", "partial"}, manifest["collection"]["status"]
outcomes = manifest["collection"]["sources"]
script_console = [item for item in outcomes if item["surface"] == "script-console"]
pipeline_gdsl = [item for item in outcomes if item["surface"] == "pipeline-gdsl"]
assert len(script_console) == 1, script_console
assert len(pipeline_gdsl) == 1 and pipeline_gdsl[0]["status"] == "succeeded", pipeline_gdsl
# Jenkins 2.356 may deny enumeration of a subset of extension classes even to an administrator.
# This is the sole accepted partial outcome; missing controller, plugin, GDSL, or library data fails.
assert script_console[0]["status"] == "succeeded" or (
    script_console[0]["status"] == "partial"
    and script_console[0].get("errorCode") == "enumeration-failed"
), script_console
unexpected = [
    item for item in outcomes
    if item["status"] not in {"succeeded", "partial"}
    or (item["status"] == "partial" and item["surface"] != "script-console")
]
assert not unexpected, unexpected
assert (manifest["collection"]["status"] == "partial") == (script_console[0]["status"] == "partial")
library_matches = [item for item in outcomes if item["surface"] == "library-source" and item["name"] == "gsa-examples"]
assert len(library_matches) == 1 and library_matches[0]["status"] == "succeeded", library_matches

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
for path in package.rglob("*"):
    if path.is_file():
        data = path.read_bytes()
        assert secret.encode() not in data, f"credential leaked into {path}"
        assert "cache" not in path.parts and "tmp" not in path.parts, path

plugin_results = json.loads(queries[0].read_text(encoding="utf-8")).get("results", [])
assert any(
    "workflow-basic-steps" in symbol.get("availability", {}).get("pluginSourceIds", [])
    or "workflow-basic-steps" in symbol.get("executionConstraints", {}).get("pluginDependencies", [])
    for symbol in plugin_results
), "Runtime returned no Pipeline symbol attributed to workflow-basic-steps"

library_results = json.loads(queries[1].read_text(encoding="utf-8"))
assert library_results.get("results"), "Runtime returned no pinned Shared Library symbols"

pipeline_steps = json.loads(queries[2].read_text(encoding="utf-8"))
step_names = {item.get("name") for item in pipeline_steps.get("steps", [])}
assert step_names.intersection({"sh", "echo", "checkout"}), step_names

print(json.dumps({
    "profile": manifest["profile"],
    "collection": manifest["collection"]["status"],
    "jenkins": controller["coreVersion"],
    "java": runtime["javaVersion"],
    "javaVendor": runtime["javaVendor"],
    "plugins": len(actual),
    "packageFiles": files,
}, indent=2))
PY

echo "Prebuilt Knowledge Package validated: $OUT"
