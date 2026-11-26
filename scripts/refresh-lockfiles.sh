#!/usr/bin/env sh
set -eu

# Run from any working directory; lockfile changes are intentional and must
# be reviewed and committed together with dependency manifest changes.
root="$(git rev-parse --show-toplevel)"
cd "$root"

if [ "$(rustc --version | awk '{print $2}')" != "1.100.0" ]; then
  echo "Rust 1.100.0 is required to refresh Cargo.lock" >&2
  exit 1
fi
if [ "$(node --version)" != "v26.13.1" ] || [ "$(npm --version)" != "11.20.0" ]; then
  echo "Node.js 26.13.1 and npm 11.20.0 are required to refresh package-lock.json" >&2
  exit 1
fi

cargo generate-lockfile
npm --prefix editors/vscode install --package-lock-only --ignore-scripts --no-audit --no-fund
