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
.devcontainer/  Reproducible VS Code Dev Container
crates/         Rust Builder, Runtime, scanners, validators, and LSP
Dockerfile.*    Separate Builder and Runtime images
schemas/        JSON Schemas for catalogs and package manifests
profiles/       Reproducible source selections
examples/       Example profiles and catalog data
scripts/        Local validation workflow
editors/vscode/ Thin VS Code extension host
```

## Design principles

1. Pin every Shared Library source to a Git commit SHA.
2. Keep controller- and plugin-specific APIs associated with their source snapshot.
3. Record provenance and confidence for every symbol.
4. Select API combinations through named profiles; do not silently mix library versions.
5. Keep runtime read-only and independent of Jenkins/Git credentials.
6. If API evidence is absent, report that it is unknown rather than inventing names or parameters.

## Implementation language and toolchain

Rust is the primary implementation language. Builder, Runtime, MCP server, CLI, catalog validation, scanners, and package tooling are implemented in Rust. Groovy is limited to Jenkins/Groovy compatibility, generated stubs, and fixtures. The VS Code extension host is a thin TypeScript layer required by the VS Code API.

The root Cargo workspace includes all seven Rust crates. The pinned stable toolchain installs `rustfmt` and `clippy`. No Gradle/JVM build system is used.

## Development environment

Open the repository in VS Code and choose **Reopen in Container**. The Dev Container provides Rust, Node.js 20, Git, and access to the host Docker engine. Its post-create step installs the VS Code extension dependencies.

Run the complete local validation from the repository root:

```sh
sh scripts/validate.sh
```

The script checks formatting, tests, Clippy, catalog schemas, a deterministic fixture package, Runtime package loading, the MCP server, Jenkins LSP, and VS Code extension compilation. Cargo outputs are written under `target/`; the fixture package is `target/fixture-package/`.

Individual workspace commands:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

## Catalog contract and validation

The canonical catalog contract is documented in [docs/catalog-contract.md](docs/catalog-contract.md). Validate catalog JSON and semantic source references with:

```sh
cargo run -p catalog-validator -- examples/catalog/catalog.json schemas/catalog.schema.json
```

## Declarative Pipeline model

The Rust Declarative Pipeline model emits schema-compatible directive symbols with parent contexts, nesting constraints, and conservative confidence. See [model usage and limitations](crates/declarative-model/README.md).

## Jenkins Shared Library scanning

Scan configured Shared Library Git revisions and emit schema-validated catalogs with exact resolved commit provenance:

```sh
cargo run -p shared-library-scanner -- libraries.json catalog.json
```

See [scanner usage and extraction limitations](crates/shared-library-scanner/README.md).

## Jenkins controller GDSL extraction

The Rust collector fetches the Pipeline DSL exposed by a configured controller and emits a schema-validated catalog. See [collector usage and compatibility notes](crates/jenkins-gdsl-collector/README.md).

## Building a Knowledge Package

Build from a profile. The checked-in fixture profile does not require Jenkins or Git access:

```sh
SOURCE_DATE_EPOCH=0 cargo run -p knowledge-builder -- build profiles/fixture.yml target/fixture-package
```

The output contains `catalog.json` and `manifest.json`. The manifest records package/catalog schema versions, profile identity, build timestamp, source revisions, source provenance, tool version, and a SHA-256 checksum of the catalog. Set `SOURCE_DATE_EPOCH=0` for a stable timestamp and byte-identical fixture package output. See [Builder documentation](crates/knowledge-builder/README.md) for live collection and credential requirements.

## Read-only Runtime CLI

A package directory contains `manifest.json` and the catalog file named by its `catalog` field (currently `catalog.json`). The Runtime validates both schemas and rejects manifest/catalog schema-version mismatches. It performs no Jenkins, Git, or network access.

```sh
cargo run -p jenkins-runtime -- --package target/fixture-package search deploy
cargo run -p jenkins-runtime -- --package target/fixture-package --profile fixture list --kind step
cargo run -p jenkins-runtime -- --package target/fixture-package get deployApp
cargo run -p jenkins-runtime -- --package target/fixture-package examples deployApp
cargo run -p jenkins-runtime -- --package target/fixture-package step sh
```

The optional `--profile` must match the package manifest; selecting another profile requires selecting its package explicitly. Results include the source snapshot, profile, and confidence.

## Read-only MCP server

Build and launch the MCP stdio server against one explicit Knowledge Package:

```sh
cargo run -p jenkins-runtime --bin jenkins-mcp -- --package ./target/fixture-package
```

The server exposes search_api, get_api_details, list_api, get_usage_examples, and get_pipeline_step, plus catalog resources. It is read-only and never connects to Jenkins or executes Pipeline steps. See [MCP server documentation](runtime/mcp.md).

## Container images

Build the images from the repository root:

```sh
docker build -f Dockerfile.builder -t jenkins-knowledge-builder .
docker build -f Dockerfile.runtime -t jenkins-knowledge-runtime .
```

The Builder image includes the compiled Builder, Git, and CA certificates for source collection. Pass credentials at runtime through environment variables or a Git credential helper; never bake them into an image. For an offline fixture build, mount the repository and an output directory:

```sh
docker run --rm \
  -v "$PWD:/workspace" \
  -v "$PWD/target:/output" \
  -w /workspace \
  jenkins-knowledge-builder \
  build profiles/fixture.yml /output/fixture-package
```

The Runtime image contains only the read-only MCP executable, schemas, and CA certificates. Mount a prebuilt package at `/package`:

```sh
docker run --rm -i -v "$PWD/target/fixture-package:/package:ro" jenkins-knowledge-runtime
```

It contains no Git client, Rust toolchain, Builder, or Jenkins credentials.

## VS Code integration

The optional VS Code extension adds Jenkins catalog completion and hover information to Groovy documents while leaving ordinary Groovy language features to installed Groovy tooling. The Rust LSP reads one explicitly selected Knowledge Package and supports Pipeline steps and Shared Library APIs.

See [VS Code setup, configuration, limitations, and smoke tests](editors/vscode/README.md). Build the server with `cargo build --release -p jenkins-lsp`.
