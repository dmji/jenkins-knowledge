#!/usr/bin/env sh
set -eu

cargo fmt --manifest-path crates/catalog-validator/Cargo.toml -- --check
cargo fmt --manifest-path crates/declarative-model/Cargo.toml -- --check
cargo fmt --all
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p catalog-validator -- examples/catalog/catalog.json schemas/catalog.schema.json

rm -rf target/fixture-package
SOURCE_DATE_EPOCH=0 cargo run -p knowledge-builder -- build profiles/fixture.yml target/fixture-package
cargo run -p catalog-validator -- target/fixture-package/catalog.json schemas/catalog.schema.json
cargo run -p jenkins-runtime -- --package target/fixture-package search deploy
cargo build -p jenkins-runtime --bin jenkins-mcp
cargo build -p jenkins-lsp
npm --prefix editors/vscode install
npm --prefix editors/vscode run compile

docker build --file Dockerfile.builder --tag jenkins-knowledge-builder:local .
docker build --file Dockerfile.runtime --tag jenkins-knowledge-runtime:local .
