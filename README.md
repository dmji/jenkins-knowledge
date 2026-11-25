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
