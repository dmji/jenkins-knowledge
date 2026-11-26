#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNNER_TEMP="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
cd "$ROOT"
export GIT_TERMINAL_PROMPT=0

OUT="$ROOT/target/jenkins-2.356"
IMAGE="jenkins-knowledge-2.356"
CONTAINER="jenkins-knowledge-2.356-${GITHUB_RUN_ID:-local}"
JENKINS_URL="http://127.0.0.1:18080"
TOKEN_FILE="$RUNNER_TEMP/jenkins-knowledge-tokens.env"
ENV_FILE="$RUNNER_TEMP/jenkins-knowledge-jenkins.env"

mkdir -p "$OUT"
rm -f "$TOKEN_FILE" "$OUT/plugin-inventory-api.json" "$OUT/environment-summary.json" "$OUT/java-version.txt" "$OUT/image.txt"
rm -rf "$OUT/quick" "$OUT/standard" "$OUT/deep" "$OUT/deep-readonly" "$OUT/cache"
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true

cleanup() {
    status=$?
    trap - EXIT
    if [ "$status" -ne 0 ]; then
        docker logs "$CONTAINER" 2>&1 | tail -n 160 || true
    fi
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    docker image rm "$IMAGE" >/dev/null 2>&1 || true
    rm -f "$TOKEN_FILE" "$ENV_FILE"
    exit "$status"
}
trap cleanup EXIT

cargo build --locked --manifest-path crates/knowledge-builder/Cargo.toml
BUILDER="$ROOT/target/debug/knowledge-builder"
test -x "$BUILDER"

admin_password="$(openssl rand -hex 32)"
reader_password="$(openssl rand -hex 32)"
cat > "$ENV_FILE" <<EOF
JAVA_OPTS=-Djenkins.install.runSetupWizard=false
TEST_ADMIN_PASSWORD=$admin_password
TEST_READER_PASSWORD=$reader_password
EOF
chmod 600 "$ENV_FILE"

docker build --platform linux/amd64 --pull=false -f tests/jenkins-2.356/Dockerfile -t "$IMAGE" tests/jenkins-2.356
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
        echo "Jenkins container stopped before creating test API tokens" >&2
        exit 1
    fi
    sleep 5
done
if [ ! -s "$TOKEN_FILE" ]; then
    echo "Timed out waiting for Jenkins test API tokens" >&2
    exit 1
fi

admin_token="$(sed -n 's/^ADMIN_TOKEN=//p' "$TOKEN_FILE")"
reader_token="$(sed -n 's/^READER_TOKEN=//p' "$TOKEN_FILE")"
test -n "$admin_token"
test -n "$reader_token"

ready=false
for attempt in $(seq 1 60); do
    if curl --silent --show-error --fail --user "admin:$admin_token" "$JENKINS_URL/api/json" >/dev/null; then
        ready=true
        break
    fi
    sleep 5
done
if [ "$ready" != "true" ]; then
    echo "Jenkins 2.356 API did not become ready" >&2
    exit 1
fi

# Prove that the reader has Overall/Read without granting Overall/Administer.
curl --silent --show-error --fail --user "reader:$reader_token" "$JENKINS_URL/api/json" >/dev/null
docker exec "$CONTAINER" java -version 2> "$OUT/java-version.txt"
grep -q '17\.' "$OUT/java-version.txt"

# Save the exact plugin inventory observed from this digest-pinned controller.
curl --silent --show-error --fail --user "admin:$admin_token" \
    "$JENKINS_URL/pluginManager/api/json?depth=1" > "$OUT/plugin-inventory.json"
printf '%s\n' 'jenkins/jenkins:2.356-jdk17@sha256:211419c659bd04b2d065ead531aad35ea47471489dcf714ab2c8a30cb9870850' \
    > "$OUT/image.txt"

run_build() {
    profile="$1"
    username="$2"
    token="$3"
    package_name="$4"
    JENKINS_USERNAME="$username" \
    JENKINS_API_TOKEN="$token" \
    JENKINS_KNOWLEDGE_CACHE_DIR="$OUT/cache/$package_name" \
        "$BUILDER" build "tests/jenkins-2.356/profiles/$profile.yml" "$OUT/$package_name"
}

run_build quick reader "$reader_token" quick
run_build standard reader "$reader_token" standard
run_build deep admin "$admin_token" deep
run_build deep-readonly reader "$reader_token" deep-readonly

python3 tests/jenkins-2.356/assert_results.py "$OUT"
rm -rf "$OUT/cache"
