#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

crates=(
  catalog-validator
  jenkins-gdsl-collector
  shared-library-scanner
  declarative-model
  knowledge-builder
  jenkins-runtime
  jenkins-lsp
)

for crate in "${crates[@]}"; do
  manifest="crates/$crate/Cargo.toml"
  echo "==> Checking $crate"

  # Preserve the current CI convention: normalize crates that opt into the
  # formatter step, then assert that the formatter has no remaining changes.
  if [[ "$crate" != "catalog-validator" ]]; then
    cargo fmt --manifest-path "$manifest"
  fi
  cargo fmt --manifest-path "$manifest" -- --check
  cargo test --manifest-path "$manifest" --all-targets
  cargo clippy --manifest-path "$manifest" --all-targets -- -D warnings
done

echo "==> Validating the checked-in example catalog"
cargo run --manifest-path crates/catalog-validator/Cargo.toml -- \
  examples/catalog/catalog.json schemas/catalog.schema.json

echo "==> Building MCP and LSP binaries"
cargo build --manifest-path crates/jenkins-runtime/Cargo.toml --bin jenkins-mcp
cargo build --manifest-path crates/jenkins-lsp/Cargo.toml

echo "==> Building an offline fixture Knowledge Package"
fixture_dir="target/fixture-package"
runtime_output="target/runtime-search.json"
rm -rf "$fixture_dir"
mkdir -p target
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}" cargo run \
  --manifest-path crates/knowledge-builder/Cargo.toml -- \
  build profiles/fixture.yml "$fixture_dir"

echo "==> Validating the generated package catalog"
cargo run --manifest-path crates/catalog-validator/Cargo.toml -- \
  "$fixture_dir/catalog.json" schemas/catalog.schema.json

echo "==> Loading the package with the Runtime CLI"
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml -- \
  --package "$fixture_dir" search deploy > "$runtime_output"
grep -q '"name": "deployApp"' "$runtime_output"

echo "==> Compiling the VS Code extension"
(
  cd editors/vscode
  npm install
  npm run compile
)

echo "All repository validation checks passed."
