#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="$ROOT/demo"
OUT="$ROOT/target/jenkins-2.356-demo"
COMPOSE=(docker compose --project-directory "$DEMO_DIR" --env-file "$DEMO_DIR/.env" -f "$DEMO_DIR/compose.yml")
MCP_COMPOSE=(docker compose --profile mcp --project-directory "$DEMO_DIR" --env-file "$DEMO_DIR/.env" -f "$DEMO_DIR/compose.yml")
JENKINS_URL="http://127.0.0.1:$(sed -n 's/^DEMO_JENKINS_PORT=//p' "$DEMO_DIR/.env" | tail -n 1)"
TOKEN_FILE=/var/jenkins_home/jenkins-knowledge-tokens.env
expected_library_commit="$(sed -n 's/^DEMO_LIBRARY_COMMIT=//p' "$DEMO_DIR/.env" | tail -n 1)"
actual_library_commit="$(git ls-remote https://github.com/GSA/jenkins-shared-library-examples.git refs/heads/main | awk 'NR == 1 { print $1 }')"
if [ "$actual_library_commit" != "$expected_library_commit" ]; then
    echo "The GSA Shared Library main branch moved after package collection; rebuild the demo against a reviewed revision." >&2
    exit 1
fi

tokens="$("${COMPOSE[@]}" exec -T jenkins cat "$TOKEN_FILE")"
admin_token="$(printf '%s\n' "$tokens" | sed -n 's/^ADMIN_TOKEN=//p')"
[ -n "$admin_token" ] || {
    echo "The local Jenkins admin token is missing." >&2
    exit 1
}

# The container healthcheck only probes /login; wait for the authenticated API too.
api_ready=false
for attempt in $(seq 1 60); do
    if curl --globoff --silent --fail --user "admin:$admin_token" \
        "$JENKINS_URL/api/json?tree=mode" >/dev/null; then
        api_ready=true
        break
    fi
    sleep 2
done
if [ "$api_ready" != "true" ]; then
    echo "The local Jenkins API did not become ready." >&2
    exit 1
fi

echo "Checking the demo Pipeline job..."
before="$(
    curl --globoff --silent --show-error --fail --user "admin:$admin_token" \
        "$JENKINS_URL/job/jenkins-knowledge-demo/api/json?tree=lastBuild[number]" |
        python3 -c 'import json,sys; print((json.load(sys.stdin).get("lastBuild") or {}).get("number") or 0)'
)"
curl --silent --show-error --fail --user "admin:$admin_token" \
    --request POST "$JENKINS_URL/job/jenkins-knowledge-demo/build" --output /dev/null

pipeline_result=""
for attempt in $(seq 1 90); do
    response="$(curl --globoff --silent --show-error --fail --user "admin:$admin_token" \
        "$JENKINS_URL/job/jenkins-knowledge-demo/lastBuild/api/json?tree=number,building,result" 2>/dev/null || true)"
    if [ -n "$response" ]; then
        read -r number building result < <(
            printf '%s' "$response" |
                python3 -c 'import json,sys; x=json.load(sys.stdin); print(x.get("number",0), str(x.get("building",True)).lower(), x.get("result") or "")'
        )
        if [ "$number" -gt "$before" ] && [ "$building" = "false" ]; then
            pipeline_result="$result"
            break
        fi
    fi
    sleep 2
done
if [ "$pipeline_result" != "SUCCESS" ]; then
    echo "Demo Pipeline did not finish successfully (result: ${pipeline_result:-timeout})." >&2
    curl --silent --show-error --user "admin:$admin_token" \
        "$JENKINS_URL/job/jenkins-knowledge-demo/lastBuild/consoleText" >&2 || true
    exit 1
fi

echo "Checking Runtime against the collected Shared Library..."
"${COMPOSE[@]}" run --rm --no-deps --entrypoint jenkins-runtime runtime \
    --package /package/knowledge-package --profile jenkins-2356-demo \
    search example1 --source-kind shared-library --limit 10 > "$OUT/runtime-search.json"
grep -q '"name": "example1"' "$OUT/runtime-search.json"

echo "Checking the MCP initialize and tools/list handshake..."
mcp_response="$(
    printf '%s\n' \
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
        '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' |
        "${MCP_COMPOSE[@]}" run --rm -T --no-deps mcp
)"
printf '%s\n' "$mcp_response" | grep -q '"protocolVersion":"2025-06-18"'
printf '%s\n' "$mcp_response" | grep -q '"name":"search_api"'

echo "Checking VS Code -> containerized LSP -> local Knowledge Package..."
npm --prefix "$ROOT/editors/vscode" ci --ignore-scripts --no-audit --no-fund
npm --prefix "$ROOT/editors/vscode" test
export JENKINS_LSP_PATH="$DEMO_DIR/bin/jenkins-lsp"
export JENKINS_KNOWLEDGE_LIVE_PACKAGE=/package/knowledge-package
export JENKINS_KNOWLEDGE_LIVE_PACKAGE_REMOTE=true
export JENKINS_KNOWLEDGE_LIVE_PROFILE=jenkins-2356-demo
export JENKINS_KNOWLEDGE_LIVE_ONLY=true
export VSCODE_TEST_VERSION=1.91.0
if command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a npm --prefix "$ROOT/editors/vscode" run test:host
else
    npm --prefix "$ROOT/editors/vscode" run test:host
fi

echo "Demo acceptance passed: Pipeline, Builder, Package, Runtime, MCP, LSP, and VS Code."
