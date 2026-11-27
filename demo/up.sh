#!/usr/bin/env bash
set -euo pipefail
umask 077

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="$ROOT/demo"
OUT="$ROOT/target/jenkins-2.356-demo"
DOCKER_CONTEXT="$OUT/docker-context"
COMPOSE=(docker compose --project-directory "$DEMO_DIR" --env-file "$DEMO_DIR/.env" -f "$DEMO_DIR/compose.yml")

for command in cargo docker git python3 openssl; do
    command -v "$command" >/dev/null 2>&1 || {
        echo "Required command not found: $command" >&2
        exit 1
    }
done
docker compose version >/dev/null

if [ ! -f "$DEMO_DIR/.env" ]; then
    printf 'DEMO_ADMIN_PASSWORD=%s\nDEMO_READER_PASSWORD=%s\nDEMO_JENKINS_PORT=18080\n' \
        "$(openssl rand -hex 24)" "$(openssl rand -hex 24)" > "$DEMO_DIR/.env"
fi
chmod 600 "$DEMO_DIR/.env"
mapfile -t library_config < <(python3 - "$DEMO_DIR/profile.yml" <<'PYPROFILE'
import re
import sys
from pathlib import Path
text = Path(sys.argv[1]).read_text(encoding="utf-8")
for key in ("url", "revision", "commit"):
    match = re.search(r"^    " + key + r":\s*(\S+)\s*$", text, re.MULTILINE)
    if not match:
        raise SystemExit(f"demo/profile.yml is missing the Shared Library {key}")
    print(match.group(1))
PYPROFILE
)
library_url="${library_config[0]}"
library_revision="${library_config[1]}"
library_commit="${library_config[2]}"
actual_library_commit="$(git ls-remote "$library_url" "refs/heads/$library_revision" | awk 'NR == 1 { print $1 }')"
if [ "$actual_library_commit" != "$library_commit" ]; then
    echo "The demo Shared Library branch no longer matches the pinned commit in demo/profile.yml." >&2
    echo "Update demo/profile.yml to a reviewed commit before starting the demo." >&2
    exit 1
fi
tmp_env="$DEMO_DIR/.env.tmp"
grep -v '^DEMO_LIBRARY_COMMIT=' "$DEMO_DIR/.env" > "$tmp_env" || true
printf 'DEMO_LIBRARY_COMMIT=%s\n' "$library_commit" >> "$tmp_env"
mv "$tmp_env" "$DEMO_DIR/.env"
chmod 600 "$DEMO_DIR/.env"
umask 022
grep -q '^DEMO_ADMIN_PASSWORD=.' "$DEMO_DIR/.env" || {
    echo "demo/.env must define DEMO_ADMIN_PASSWORD" >&2
    exit 1
}
grep -q '^DEMO_READER_PASSWORD=.' "$DEMO_DIR/.env" || {
    echo "demo/.env must define DEMO_READER_PASSWORD" >&2
    exit 1
}

echo "Building the pinned Rust binaries..."
cargo build --locked --release --workspace --bins

echo "Preparing the digest-pinned Jenkins 2.356 plugin image..."
rm -rf "$DOCKER_CONTEXT"
mkdir -p "$DOCKER_CONTEXT/plugins"
cp "$ROOT/tests/jenkins-2.356/Dockerfile" "$DOCKER_CONTEXT/Dockerfile"
python3 "$ROOT/tests/jenkins-2.356/test_resolve_plugins.py"
python3 "$ROOT/tests/jenkins-2.356/resolve_plugins.py" \
    --lock-file "$ROOT/tests/jenkins-2.356/plugins.lock.json" \
    --output "$DOCKER_CONTEXT/plugins" \
    --lock-output "$OUT/plugins.lock.json"

"${COMPOSE[@]}" build jenkins builder runtime
"${COMPOSE[@]}" up -d --wait --wait-timeout 600 jenkins

echo "Jenkins is ready at http://127.0.0.1:$(sed -n 's/^DEMO_JENKINS_PORT=//p' "$DEMO_DIR/.env" | tail -n 1)"
echo "Demo-only credentials are stored in demo/.env (mode 0600)."
echo "Next: bash demo/build-package.sh"
