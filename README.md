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

Open the repository in VS Code and choose **Reopen in Container**. The Dev Container provides Rust 1.100.0, Node.js 26.13.1 (npm 11.20.0), Git, and access to the host Docker engine. Its post-create step installs the VS Code extension dependencies.

Run the complete local validation from the repository root:

```sh
sh scripts/validate.sh
```

The script checks formatting without rewriting source. New Rust files and changed files whose base revision is already formatted are checked; pre-existing formatting drift in a legacy file is reported and preserved rather than silently rewritten. A deliberately unformatted fixture verifies that the read-only formatter check fails. The script also runs tests, Clippy, catalog schemas, deterministic fixture generation (two consecutive builds are compared), Runtime package loading, the MCP server, Jenkins LSP, VS Code extension compilation, both container image builds, and the Builder-to-Runtime handoff. It verifies the checked-out commit SHA and fails if validation changes tracked or untracked working-tree files. Run it from a clean checkout. Cargo outputs are written under `target/`; the fixture package is `target/fixture-package/`.

Individual workspace commands:

```sh
cargo fmt --all -- --check
cargo --locked test --workspace --all-targets
cargo --locked clippy --workspace --all-targets -- -D warnings
```

## Reproducible dependencies and lockfile policy

The repository commits the workspace `Cargo.lock` and VS Code `editors/vscode/package-lock.json`. CI and the Dev Container install the exact locked dependency graph (`cargo --locked` and `npm ci`); validation must not create, rewrite, or silently commit lockfiles.

Use the explicit refresh command when intentionally updating dependency resolution:

```sh
sh scripts/refresh-lockfiles.sh
```

Review both lockfile diffs with the manifest changes, then run the full validation before opening a dependency-update PR. Dependabot checks Cargo, npm, and GitHub Actions weekly. Workflow actions are pinned to immutable upstream commit SHAs with their release tags recorded as comments. The Rust compiler and Node/npm versions are pinned in `rust-toolchain.toml` and the workflow/Dev Container configuration.

## Catalog contract and validation

The canonical catalog contract is documented in [docs/catalog-contract.md](docs/catalog-contract.md). Validate catalog JSON and semantic source references with:

```sh
cargo --locked run -p catalog-validator -- examples/catalog/catalog.json schemas/catalog.schema.json
```

## Declarative Pipeline model

The Rust Declarative Pipeline model emits schema-compatible directive symbols with parent contexts, nesting constraints, and conservative confidence. See [model usage and limitations](crates/declarative-model/README.md).

## Jenkins Shared Library scanning

Scan configured Shared Library Git revisions and emit schema-validated catalogs with exact resolved commit provenance:

```sh
cargo --locked run -p shared-library-scanner -- libraries.json catalog.json
```

See [scanner usage and extraction limitations](crates/shared-library-scanner/README.md).

## Jenkins controller GDSL extraction

The Rust collector fetches the Pipeline DSL exposed by a configured controller and emits a schema-validated catalog. See [collector usage and compatibility notes](crates/jenkins-gdsl-collector/README.md).

## Building a Knowledge Package

Build from a profile. The checked-in fixture profile does not require Jenkins or Git access:

```sh
SOURCE_DATE_EPOCH=0 cargo --locked run -p knowledge-builder -- build profiles/fixture.yml target/fixture-package
```

The output contains `catalog.json` and `manifest.json`. Profiles may set `collectionProfile: quick|standard|deep` (default `standard`). The manifest records package/catalog schema versions, profile identity, build timestamp, source revisions, source provenance, tool version, a SHA-256 checksum of the catalog, and explicit package-level/per-source collection outcomes. Optional-source failures preserve successful data and mark the package partial; mandatory controller metadata failure aborts the build. The `checksums` object is the complete set of package files covered by integrity verification (currently `catalog.json`); `manifest.json` is not self-checksummed. Runtime verifies every declared file before parsing the catalog and rejects missing files, invalid digests, duplicate entries, unsafe paths, and symbolic links. Set `SOURCE_DATE_EPOCH=0` for a stable timestamp and byte-identical fixture package output. See [Builder documentation](crates/knowledge-builder/README.md) for live collection and credential requirements.

## Read-only Runtime CLI

A package directory contains `manifest.json` and the catalog file named by its `catalog` field (currently `catalog.json`). Runtime embeds the manifest and catalog schemas in the executable, verifies every file listed in `manifest.checksums` with SHA-256 before parsing the catalog, and rejects packages with missing or mismatched files, malformed or duplicate checksum entries, unsafe paths, or symbolic links. Schema loading and package validation are independent of the process working directory. It performs no Jenkins, Git, or network access.

```sh
cargo --locked run -p jenkins-runtime -- --package target/fixture-package search deploy
cargo --locked run -p jenkins-runtime -- --package target/fixture-package --profile fixture list --kind step
cargo --locked run -p jenkins-runtime -- --package target/fixture-package get deployApp
cargo --locked run -p jenkins-runtime -- --package target/fixture-package examples deployApp
cargo --locked run -p jenkins-runtime -- --package target/fixture-package step sh
```

The optional `--profile` must match the package manifest; selecting another profile requires selecting its package explicitly. Results include the source snapshot, profile, and confidence.

## Read-only MCP server

Build and launch the MCP stdio server against one explicit Knowledge Package:

```sh
cargo --locked run -p jenkins-runtime --bin jenkins-mcp -- --package ./target/fixture-package
```

The server exposes search_api, get_api_details, list_api, get_usage_examples, and get_pipeline_step, plus catalog resources. It is read-only and never connects to Jenkins or executes Pipeline steps. See [MCP server documentation](runtime/mcp.md).

## Container images

Build the Builder and Runtime release binaries once before packaging either image. Both images consume these same workspace artifacts; Docker does not compile Rust:

```sh
cargo --locked build --release -p knowledge-builder -p jenkins-runtime
docker build -f Dockerfile.builder -t jenkins-knowledge-builder .
docker build -f Dockerfile.runtime -t jenkins-knowledge-runtime .
```

The Docker build context includes only the two required release executables from `target/release/`; other Cargo build artifacts remain excluded by `.dockerignore`.

The Builder image includes the compiled Builder, Git, and CA certificates for source collection. Pass credentials at runtime through environment variables or a Git credential helper; never bake them into an image. For an offline fixture build, mount the repository and an output directory:

```sh
docker run --rm \
  -v "$PWD:/workspace" \
  -v "$PWD/target:/output" \
  -w /workspace \
  jenkins-knowledge-builder \
  build profiles/fixture.yml /output/fixture-package
```

The Runtime image contains only the read-only MCP executable with embedded schemas and CA certificates. Mount a prebuilt package at `/package`:

```sh
docker run --rm -i -v "$PWD/target/fixture-package:/package:ro" jenkins-knowledge-runtime
```

It contains no Git client, Rust toolchain, Builder, or Jenkins credentials.

## VS Code integration

The optional VS Code extension adds Jenkins catalog completion and hover information to Groovy documents while leaving ordinary Groovy language features to installed Groovy tooling. The Rust LSP reads one explicitly selected Knowledge Package and supports Pipeline steps and Shared Library APIs.

See [VS Code setup, configuration, limitations, and smoke tests](editors/vscode/README.md). Build the server with `cargo --locked build --release -p jenkins-lsp`.
