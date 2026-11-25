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

The Rust Builder assembles validated, versioned Knowledge Packages from profile-selected Jenkins and Shared Library sources. Offline fixture builds are supported for CI and reproducibility checks.

## Implementation language

Rust is the primary implementation language. Builder, Runtime, MCP server, CLI, catalog validation, scanners, and package tooling must be implemented in Rust.

- VS Code language/intelligence backend and LSP: Rust. A thin extension host may use TypeScript where required by the VS Code extension API; keep project logic in Rust.
- Groovy is limited to Jenkins/Groovy compatibility, generated Groovy stubs, and test fixtures. It is not a core implementation language.
- Cargo is the standard build and test system. Do not introduce Gradle/JVM as the project build system.
- Prefer established Rust crates and the standard library. Document and justify dependencies and any technically necessary language exceptions.

## Development environment and validation

Open the repository in VS Code and choose **Dev Containers: Reopen in Container**. The container provides stable Rust with rustfmt and Clippy, Node.js 20, npm, and Git. The post-create check verifies that these tools are available.

Run the canonical validation script from the repository root:

    bash scripts/validate.sh

It checks formatting, tests, and Clippy for every Rust crate; validates the example catalog; builds the MCP and LSP binaries; builds and validates an offline fixture package; opens that package with the Runtime CLI; and compiles the VS Code extension.

The same script is run by CI inside the Dev Container. CI also builds both production images and tests the Builder-to-Runtime package handoff.

The crates intentionally retain independent Cargo manifests rather than introducing a root workspace. The validation script enumerates all seven crates explicitly, preserving their independent build boundaries and avoiding a partial workspace definition. Use the script instead of maintaining a second set of ad hoc CI commands.

## Builder and Runtime container images

The Builder and Runtime are separate images with different responsibilities.

Build the Builder image:

    docker build -f containers/builder/Dockerfile -t jenkins-knowledge-builder:local .

Run an offline fixture build (the mounted repository is the working directory):

    docker run --rm \
      --user "$(id -u):$(id -g)" \
      --env HOME=/tmp \
      --volume "$PWD:/workspace" \
      --workdir /workspace \
      jenkins-knowledge-builder:local \
      build profiles/fixture.yml target/fixture-package

The Builder image contains the compiled package builder and catalog validator, Git, OpenSSH client, and CA certificates. Live collection credentials are supplied only when the container runs, for example through JENKINS_USERNAME, JENKINS_API_TOKEN, a mounted Git credential configuration, or an SSH agent/key. Do not put credentials in Dockerfiles, image build arguments, or the image itself.

Build and run the credential-free Runtime image:

    docker build -f containers/runtime/Dockerfile -t jenkins-knowledge-runtime:local .
    docker run --rm \
      --volume "$PWD/target/fixture-package:/package:ro" \
      jenkins-knowledge-runtime:local \
      --package /package search deploy

The Runtime image contains only the Runtime CLI, MCP server, Groovy stub generator, and the schemas needed to load a package. It runs as a non-root user and intentionally excludes Git, Cargo/Rust tooling, Builder executables, and Jenkins/Git credentials. Mount a completed package read-only at /package; the Runtime performs no collection or network access.

To launch the MCP stdio server from the same image:

    docker run --rm -i \
      --volume "$PWD/target/fixture-package:/package:ro" \
      --entrypoint jenkins-mcp \
      jenkins-knowledge-runtime:local \
      --package /package

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

## Building a Knowledge Package

Build from a profile (the checked-in fixture profile does not require Jenkins or Git access):

```sh
cargo test --manifest-path crates/knowledge-builder/Cargo.toml --all-targets
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- build profiles/fixture.yml ./target/fixture-package
```

The output contains `catalog.json` and `manifest.json`. See [Builder documentation](crates/knowledge-builder/README.md) for live collection, provenance, checksums, and reproducible timestamp settings.

## Read-only Runtime CLI

A package directory contains `manifest.json` and the catalog file named by its `catalog` field (currently `catalog.json`). The Runtime validates both schemas and rejects manifest/catalog schema-version mismatches. Run commands from the repository root so the bundled schemas can be found.

```sh
cargo test --manifest-path crates/jenkins-runtime/Cargo.toml --all-targets
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml -- --package path/to/package search deploy
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml -- --package path/to/package --profile production list --kind step
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml -- --package path/to/package get deployApp
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml -- --package path/to/package examples deployApp
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml -- --package path/to/package step sh
```

The optional `--profile` must match the package manifest; selecting another profile requires selecting its package explicitly. Results include the source snapshot, profile, and confidence. The Runtime performs no Jenkins/Git/network access and exposes no write operations.

## Read-only MCP server

Build and launch the MCP stdio server against one explicit Knowledge Package:

```sh
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml --bin jenkins-mcp -- --package ./target/fixture-package
```

The server exposes search_api, get_api_details, list_api, get_usage_examples, and get_pipeline_step, plus the jenkins://catalog/manifest, jenkins://catalog/libraries, jenkins://catalog/pipeline-steps, and selected-profile resources. It is read-only and never connects to Jenkins or executes Pipeline steps. See [MCP server documentation](runtime/mcp.md) for client configuration and protocol details.

## VS Code integration

The optional VS Code extension adds Jenkins catalog completion and hover information to Groovy documents while leaving ordinary Groovy language features to the installed Groovy tooling. The Rust LSP reads one explicitly selected Knowledge Package and supports Pipeline steps and Shared Library APIs.

See [VS Code setup, configuration, limitations, and smoke tests](editors/vscode/README.md).
Build the server with `cargo build --release --manifest-path crates/jenkins-lsp/Cargo.toml`.
