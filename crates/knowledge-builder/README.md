# Knowledge Builder

Build a versioned Knowledge Package from an explicit YAML profile.

## Offline fixture build

The profile may provide `fixtures`, a list of catalog JSON paths relative to the profile file. This mode is intended for tests and reproducible builds; it does not contact Jenkins or Git.

```sh
cargo test --manifest-path crates/knowledge-builder/Cargo.toml --all-targets
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- build crates/knowledge-builder/tests/fixtures/profile.yml ./target/fixture-package
```

## Live collection

Without `fixtures`, the Builder runs the configured Jenkins GDSL collectors and Shared Library scanners, then adds the conservative Declarative Pipeline model for each controller. Jenkins credentials are read from `JENKINS_USERNAME` and `JENKINS_API_TOKEN`; provide both or neither. Git credentials use the normal Git credential helper or SSH agent. Never embed credentials in profile URLs. Controller URLs must use HTTP(S), and Git remotes must use HTTPS, SSH, or Git SCP syntax. URL user information, query parameters, and fragments are rejected; Git references that could be interpreted as command-line options or malformed refs are rejected before any network operation.

Shared Library `commit` values, when supplied, must be full 40-character commit SHAs. The scanner records the resolved commit in the catalog source revision. When omitted, the configured ref is fetched and the resolved commit is recorded by the scanner.

## Package layout and guarantees

The output directory contains `catalog.json` and `manifest.json`. The manifest records package/catalog schema versions, profile identity, build timestamp, source revisions, source provenance, tool version, and a SHA-256 checksum of the catalog. Catalog sources and symbols are sorted by ID before serialization.

Set `SOURCE_DATE_EPOCH=0` for a stable timestamp and byte-identical fixture package output. Builds fail with the source label on collection or validation errors. The Builder writes into a unique temporary directory on the output filesystem and publishes it with a same-filesystem rename only after validation and serialization. Temporary checkouts and staging directories are managed by RAII and are removed on every early return, including failed Git operations and serialization/write errors. It refuses to overwrite an existing output directory. Source URLs are sanitized before entering catalogs and manifests so user information and URL request modifiers are not persisted.

```sh
cargo test --manifest-path crates/knowledge-builder/Cargo.toml --all-targets
cargo clippy --manifest-path crates/knowledge-builder/Cargo.toml --all-targets -- -D warnings
```
