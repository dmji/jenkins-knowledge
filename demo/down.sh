#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="$ROOT/demo"
COMPOSE=(docker compose --project-directory "$DEMO_DIR" --env-file "$DEMO_DIR/.env" -f "$DEMO_DIR/compose.yml")

if [ "${1:-}" = "--reset" ]; then
    if [ -f "$DEMO_DIR/.env" ]; then
        "${COMPOSE[@]}" down --volumes --remove-orphans
    else
        DEMO_ADMIN_PASSWORD=unused DEMO_READER_PASSWORD=unused DEMO_LIBRARY_COMMIT=unused \
            docker compose --project-directory "$DEMO_DIR" -f "$DEMO_DIR/compose.yml" down --volumes --remove-orphans
    fi
    docker image rm jenkins-knowledge-demo:2.356 jenkins-knowledge-builder:demo jenkins-knowledge-runtime:demo >/dev/null 2>&1 || true
    rm -f "$DEMO_DIR/.env" "$DEMO_DIR/.env.tmp"
    rm -rf "$ROOT/target/jenkins-2.356-demo"
    echo "Demo containers, volumes, generated credentials, and local images removed."
else
    if [ ! -f "$DEMO_DIR/.env" ]; then
        echo "Run bash demo/up.sh first." >&2
        exit 1
    fi
    "${COMPOSE[@]}" down --remove-orphans
    echo "Demo containers stopped. Jenkins data, package, and demo credentials are preserved."
    echo "Use bash demo/down.sh --reset to remove all demo data."
fi
