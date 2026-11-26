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

## Profile contract

Profiles use strict YAML deserialization: unknown keys at the root or in any nested configuration object are errors. This prevents spelling mistakes from being silently ignored.

| Field | Default / requiredness | Effect |
|---|---|---|
| `profile` | Required, non-empty | Profile identity written to `manifest.json`; used as the catalog ID when `package.id` is omitted. |
| `jenkins.controllers` | Empty when omitted; at most one supported | The controller ID scopes source identity and its URL is the HTTP(S) endpoint. Multiple controllers are rejected before collection because the current catalog schema has one top-level controller snapshot. |
| `libraries` | Empty when omitted | Each Shared Library is scanned once. `name`, `url`, and `revision` are required; optional `commit` pins the fetch to a full 40-character SHA while retaining the configured revision as metadata. Duplicate names are rejected. |
| `fixtures` | Empty when omitted | Offline catalog JSON paths, relative to the profile file. Fixture mode is mutually exclusive with live controllers/libraries. Duplicate and absolute paths are rejected. |
| `package.id` | Omitted | Catalog ID defaults to `profile`; a supplied ID must be non-empty and have no surrounding whitespace. |
| `package.include` | Omitted means all sections | Selects package sections before merge/serialization. An explicit empty list, duplicates, unknown values, and selectors matching no sources are errors. |

Supported `package.include` values:

- `jenkins-core`: Jenkins Core version source observed from each configured controller (or explicit `jenkins-core` sources in fixtures).
- `controller`: controller source, controller snapshot, and Declarative model sources.
- `plugins`: plugin sources, their symbols, and the controller's plugin inventory.
- `shared-libraries`: Shared Library sources and symbols.

Filtering is applied by source ID: excluded sources, their symbols, and their coverage entries are removed together. When `controller` is selected without `plugins`, the controller snapshot is retained but its nested plugin inventory is empty. Omit `package.include` to include every available section; `include: []` is not an alias for the default.

**Migration note:** earlier Builder versions accepted `package.include` but ignored it. It now actively filters the package. To preserve the previous all-sections output, remove `package.include` rather than setting it to an empty list. Review existing explicit selectors before upgrading; every requested section must match at least one source.

A profile must select at least one source mode. Fixture catalogs cannot be mixed with live collection. Controller IDs and library names must be unique. URL, ref, commit, fixture-path, package-ID, and selector validation completes before any external collection starts. A requested section must match at least one source after collection; an unavailable plugin inventory therefore fails an explicit `plugins` selection instead of silently producing a partial package.

## Package layout and guarantees

The output directory contains `catalog.json` and `manifest.json`. The manifest records package/catalog schema versions, profile identity, build timestamp, source revisions, source provenance, tool version, and a SHA-256 checksum of the catalog. Catalog sources and symbols are sorted by ID before serialization.

Set `SOURCE_DATE_EPOCH=0` for a stable timestamp and byte-identical fixture package output. Builds fail with the source label on collection or validation errors. The Builder writes into a unique temporary directory on the output filesystem and publishes it with a same-filesystem rename only after validation and serialization. Temporary checkouts and staging directories are managed by RAII and are removed on every early return, including failed Git operations and serialization/write errors. It refuses to overwrite an existing output directory. Controller snapshot identity, Core version, plugin inventory, capabilities, and collection metadata are retained; builds reject incompatible controller snapshots instead of silently combining them. Source URLs are sanitized before entering catalogs and manifests so user information and URL request modifiers are not persisted.

```sh
cargo test --manifest-path crates/knowledge-builder/Cargo.toml --all-targets
cargo clippy --manifest-path crates/knowledge-builder/Cargo.toml --all-targets -- -D warnings
```
