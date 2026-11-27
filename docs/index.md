# Jenkins Knowledge documentation

Jenkins Knowledge builds a versioned knowledge package from an existing Jenkins controller and optional Shared Libraries. Runtime, MCP, and VS Code consumers use that package locally without connecting to Jenkins or Git.

## Start here

- [Reference deployment guide](reference-deployment/index.md) — production lifecycle, starting with an existing Jenkins.
- [Prerequisites and compatibility](reference-deployment/prerequisites.md)
- [Connect a controller](reference-deployment/integration/controller.md)
- [Collection profiles](reference-deployment/integration/collection.md)
- [Package lifecycle and operations](reference-deployment/operations/refresh.md)
- [Acceptance checklist](reference-deployment/acceptance-tests.md)
- [Optional Jenkins 2.356 demo](reference-deployment/demo-environment/installation.md)

## Technical reference

- [Architecture](architecture.md) · [Catalog contract](catalog-contract.md)
- [Builder](../crates/knowledge-builder/README.md) · [MCP](../runtime/mcp.md) · [VS Code](../editors/vscode/README.md)

The production guide does not require Docker Compose or a new Jenkins installation. The demo is separate.
