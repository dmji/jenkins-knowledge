#!/usr/bin/env sh
set -eu

# Run from any working directory; lockfile changes are intentional and must
# be reviewed and committed together with dependency manifest changes.
root="$(git rev-parse --show-toplevel)"
cd "$root"

cargo generate-lockfile
npm --prefix editors/vscode install --package-lock-only --ignore-scripts --no-audit --no-fund
