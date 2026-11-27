# Jenkins Knowledge documentation

Jenkins Knowledge builds a versioned knowledge package from an existing Jenkins controller and optional Shared Libraries. Runtime, MCP, and VS Code consumers use that package locally without connecting to Jenkins or Git.

## Start here

- [Reference deployment guide](reference-deployment/index.md) — production lifecycle, starting with an existing Jenkins.
- [Prerequisites and compatibility](reference-deployment/prerequisites.md)
- [Acceptance checklist](reference-deployment/acceptance-tests.md)

## Integration

- [Connect an existing controller](reference-deployment/integration/controller.md)
- [Jenkins permissions](reference-deployment/integration/permissions.md)
- [Credentials](reference-deployment/integration/credentials.md)
- [Shared Libraries](reference-deployment/integration/shared-libraries.md)
- [Collection profiles](reference-deployment/integration/collection.md)
- [Knowledge Package](reference-deployment/integration/package.md)
- [Runtime CLI](reference-deployment/integration/runtime.md)
- [MCP server](reference-deployment/integration/mcp.md)
- [VS Code](reference-deployment/integration/vscode.md)

## Operations

- [Refresh](reference-deployment/operations/refresh.md)
- [Versioning and promotion](reference-deployment/operations/versioning.md)
- [Backup and restore](reference-deployment/operations/backup-restore.md)
- [Troubleshooting](reference-deployment/operations/troubleshooting.md)

## Optional demo

- [Installation](reference-deployment/demo-environment/installation.md)
- [Configuration](reference-deployment/demo-environment/configuration.md)
- [Test data](reference-deployment/demo-environment/test-data.md)
- [Walkthrough](reference-deployment/demo-environment/walkthrough.md)

## Technical reference

- [Architecture](architecture.md) · [Catalog contract](catalog-contract.md)
- [Builder](https://github.com/dmji/jenkins-knowledge/blob/master/crates/knowledge-builder/README.md) · [MCP](https://github.com/dmji/jenkins-knowledge/blob/master/runtime/mcp.md) · [VS Code](https://github.com/dmji/jenkins-knowledge/blob/master/editors/vscode/README.md)

The production guide does not require Docker Compose or a new Jenkins installation. The demo is separate.

## Publishing

The documentation is built from `docs/` and deployed by the **Documentation Pages** workflow. In repository Settings → Pages, set the build and deployment source to **GitHub Actions**. Pull requests build the site without deploying it.
