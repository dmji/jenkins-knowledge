#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

OUT="$ROOT/target/jenkins-2.356"
BUILDER="$ROOT/target/debug/knowledge-builder"
LSP="$ROOT/target/release/jenkins-lsp"
PRIMARY="$OUT/contract-primary"
ALTERNATE="$OUT/contract-alternate"

test -s "$OUT/deep/catalog.json"
test -s "$OUT/deep/manifest.json"
test -x "$BUILDER"

cargo build --locked --release -p jenkins-lsp
rm -rf "$PRIMARY" "$ALTERNATE"
"$BUILDER" build crates/jenkins-contract-tests/tests/fixtures/primary/profile.yml "$PRIMARY"
"$BUILDER" build crates/jenkins-contract-tests/tests/fixtures/alternate/profile.yml "$ALTERNATE"

cd "$ROOT/editors/vscode"
npm ci --ignore-scripts --no-audit --no-fund
npm test

export JENKINS_KNOWLEDGE_LIVE_PACKAGE="$OUT/deep"
export JENKINS_KNOWLEDGE_PRIMARY_PACKAGE="$PRIMARY"
export JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE="$ALTERNATE"
export JENKINS_LSP_PATH="$LSP"
export VSCODE_TEST_VERSION=1.91.0
xvfb-run -a npm run test:host
