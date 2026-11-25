# Shared Library scanner

Scans pinned Jenkins Shared Library Git revisions and emits canonical catalog JSON plus a diagnostics file. Requires Rust/Cargo and Git. It never executes Groovy code.

## Configuration

\`\`\`json
{
  "schema_version": "1.0.0",
  "catalog_id": "team-ci",
  "libraries": [
    {
      "name": "platform",
      "repository": "https://github.com/acme/platform-library.git",
      "reference": "refs/tags/v2.1.0"
    }
  ]
}
\`\`\`

Repository URLs must not contain embedded credentials. Use Git credential helpers, SSH agents, or the CI environment for authentication. Git command output is deliberately suppressed on failure because it can contain credential-bearing URLs.

## Run

\`\`\`sh
cargo test --manifest-path crates/shared-library-scanner/Cargo.toml
cargo run --manifest-path crates/shared-library-scanner/Cargo.toml -- config.json catalog.json diagnostics.json
cargo run --manifest-path crates/catalog-validator/Cargo.toml -- catalog.json schemas/catalog.schema.json
\`\`\`

Each configured library/ref creates its own source. `revision` and `metadata.resolvedCommitSha` contain the resolved 40-character commit SHA; `version` and `metadata.configuredRef` preserve the requested ref. The scanner walks Groovy files below `vars/` and `src/`, creates global-variable symbols for `vars/*.groovy`, and extracts declaration-shaped methods and classes. Overloads remain separate symbols because their source locations differ.

The scanner is intentionally heuristic: extracted symbols use `inferred` confidence. It does not execute Groovy, resolve inheritance, infer runtime types/defaults, or claim arbitrary dynamic dispatch. Dynamic dispatch hooks are emitted to diagnostics JSON. Multiline declarations, complex generic signatures, and some valid Groovy syntax may not be recognized. Documentation in adjacent `vars/<name>.txt` files is attached to the global symbol when present.
