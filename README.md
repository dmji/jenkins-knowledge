# Jenkins Knowledge

Build and distribute verified Jenkins Pipeline API knowledge for IDEs and AI agents.

## Architecture

- **Builder** scans Jenkins controllers, plugin metadata, and pinned Shared Library revisions.
- **Catalog** is the canonical, versioned JSON representation of discovered symbols.
- **Runtime** reads a built package and exposes read-only search, details, examples, and pipeline-step lookup.
- **Profiles** select an explicit combination of controller snapshots and library revisions.

Generated Groovy stubs, MCP resources, and documentation are derived from the catalog; they are not the source of truth.

## Repository layout

```text
builder/       Extraction and package generation
runtime/       Read-only catalog access and MCP interface
schemas/       JSON Schemas for catalogs and package manifests
profiles/      Reproducible source selections
examples/      Example profiles and catalog data
```

## Design principles

1. Pin every Shared Library source to a Git commit SHA.
2. Keep controller- and plugin-specific APIs associated with their source snapshot.
3. Record provenance and confidence for every symbol.
4. Select API combinations through named profiles; do not silently mix library versions.
5. Keep runtime read-only and independent of Jenkins/Git credentials.
6. If API evidence is absent, report that it is unknown rather than inventing names or parameters.

## Initial milestone

Define and validate the catalog and package contracts first. Add scanners, runtime interfaces, MCP transport, and Groovy stub generation incrementally against real fixtures.

## Status

Early scaffold. Extraction and runtime implementations are not yet complete.

## Implementation language

Rust is the primary implementation language. Builder, Runtime, MCP server, CLI, catalog validation, scanners, and package tooling must be implemented in Rust.

- VS Code language/intelligence backend and LSP: Rust. A thin extension host may use TypeScript where required by the VS Code extension API; keep project logic in Rust.
- Groovy is limited to Jenkins/Groovy compatibility, generated Groovy stubs, and test fixtures. It is not a core implementation language.
- Cargo is the standard build and test system. Do not introduce Gradle/JVM as the project build system.
- Prefer established Rust crates and the standard library. Document and justify dependencies and any technically necessary language exceptions.

## Catalog contract and validation

The canonical catalog contract is documented in [docs/catalog-contract.md](docs/catalog-contract.md). Validate catalog JSON and semantic source references with the Rust validator:

```sh
cargo test --manifest-path crates/catalog-validator/Cargo.toml
cargo run --manifest-path crates/catalog-validator/Cargo.toml -- examples/catalog/catalog.json schemas/catalog.schema.json
```

## Declarative Pipeline model

The Rust Declarative Pipeline model emits schema-compatible directive symbols with parent contexts, nesting constraints, and conservative confidence. See [model usage and limitations](crates/declarative-model/README.md).

```sh
cargo test --manifest-path crates/declarative-model/Cargo.toml --all-targets
```

## Jenkins Shared Library scanning

Scan configured Shared Library Git revisions and emit schema-validated catalogs with exact resolved commit provenance:

```sh
cargo test --manifest-path crates/shared-library-scanner/Cargo.toml
cargo run --manifest-path crates/shared-library-scanner/Cargo.toml -- libraries.json catalog.json
```

See [scanner usage and extraction limitations](crates/shared-library-scanner/README.md).

## Jenkins controller GDSL extraction

The Rust collector fetches the Pipeline DSL exposed by a configured controller and emits a schema-validated catalog. See [collector usage and compatibility notes](crates/jenkins-gdsl-collector/README.md).
