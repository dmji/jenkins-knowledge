#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="$ROOT/demo"
COMPOSE=(docker compose --project-directory "$DEMO_DIR" --env-file "$DEMO_DIR/.env" -f "$DEMO_DIR/compose.yml")
TOKEN_FILE=/var/jenkins_home/jenkins-knowledge-tokens.env

if [ ! -f "$DEMO_DIR/.env" ]; then
    echo "Run bash demo/up.sh first." >&2
    exit 1
fi

reader_token=""
for attempt in $(seq 1 120); do
    if tokens="$("${COMPOSE[@]}" exec -T jenkins cat "$TOKEN_FILE" 2>/dev/null)"; then
        reader_token="$(printf '%s\n' "$tokens" | sed -n 's/^READER_TOKEN=//p')"
        [ -n "$reader_token" ] && break
    fi
    sleep 2
done
if [ -z "$reader_token" ]; then
    echo "Timed out waiting for the local Jenkins reader API token." >&2
    exit 1
fi

echo "Building the Jenkins Knowledge Package from the local controller..."
"${COMPOSE[@]}" run --rm --no-deps --entrypoint sh builder \
    -c 'rm -rf /packages/knowledge-package'
export JENKINS_USERNAME=reader
export JENKINS_API_TOKEN="$reader_token"
export JENKINS_KNOWLEDGE_CACHE_DIR=/tmp/jenkins-knowledge-demo-cache
"${COMPOSE[@]}" run --rm --no-deps \
    -e JENKINS_USERNAME -e JENKINS_API_TOKEN -e JENKINS_KNOWLEDGE_CACHE_DIR builder
unset JENKINS_API_TOKEN JENKINS_USERNAME

"${COMPOSE[@]}" up -d --wait --wait-timeout 60 runtime

echo "Package ready in the shared Docker volume at /package/knowledge-package."
echo "Next: bash demo/verify.sh"
