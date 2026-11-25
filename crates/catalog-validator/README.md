# Catalog validator

Validate a catalog against the repository schema and semantic reference rules.

```sh
cargo test --manifest-path crates/catalog-validator/Cargo.toml
cargo run --manifest-path crates/catalog-validator/Cargo.toml -- \
  examples/catalog/catalog.json schemas/catalog.schema.json
```

Exit codes: `0` valid, `1` catalog validation failed, `2` input/usage error.
