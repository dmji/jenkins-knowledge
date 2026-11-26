#!/usr/bin/env sh
set -eu

generated_lockfile=0
if [ ! -f Cargo.lock ]; then
  generated_lockfile=1
fi
cleanup() {
  if [ "$generated_lockfile" -eq 1 ]; then
    rm -f Cargo.lock
  fi
}
trap cleanup EXIT

expected_sha="${GITHUB_SHA:-$(git rev-parse HEAD)}"
actual_sha="$(git rev-parse HEAD)"
printf 'Validating commit %s\n' "$actual_sha"
if [ "$actual_sha" != "$expected_sha" ]; then
  echo "checkout SHA does not match expected SHA $expected_sha" >&2
  exit 1
fi

cargo fmt --manifest-path crates/catalog-validator/Cargo.toml -- --check
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p catalog-validator -- examples/catalog/catalog.json schemas/catalog.schema.json

rm -rf target/fixture-package target/fixture-package-repeat
SOURCE_DATE_EPOCH=0 cargo run -p knowledge-builder -- build profiles/fixture.yml target/fixture-package
SOURCE_DATE_EPOCH=0 cargo run -p knowledge-builder -- build profiles/fixture.yml target/fixture-package-repeat
diff -ru target/fixture-package target/fixture-package-repeat
cargo run -p catalog-validator -- target/fixture-package/catalog.json schemas/catalog.schema.json
cargo run -p jenkins-runtime -- --package target/fixture-package search deploy
cargo build -p jenkins-runtime --bin jenkins-mcp
cargo build -p jenkins-lsp
npm --prefix editors/vscode install --no-save --package-lock=false
npm --prefix editors/vscode run compile

cargo build --release -p knowledge-builder -p catalog-validator -p jenkins-runtime --bins
docker build --file Dockerfile.builder --tag jenkins-knowledge-builder:local .
docker build --file Dockerfile.runtime --tag jenkins-knowledge-runtime:local .
BUILDER_IMAGE=jenkins-knowledge-builder:local RUNTIME_IMAGE=jenkins-knowledge-runtime:local bash .github/scripts/container-smoke.sh

# Remove Cargo's transient lockfile only when the checkout did not provide one.
cleanup
trap - EXIT

# Validation must leave the checked-out commit untouched.
git diff --exit-code HEAD --
if [ -n "$(git status --porcelain --untracked-files=all)" ]; then
  echo "validation left untracked or modified files in the working tree:" >&2
  git status --short >&2
  exit 1
fi
