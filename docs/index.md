# Jenkins Knowledge documentation

Jenkins Knowledge builds a versioned knowledge package from an existing Jenkins controller and optional Shared Libraries. Runtime, MCP, and VS Code consumers use that package locally without connecting to Jenkins or Git.

## Start here

- [Reference deployment guide]({{ '/reference-deployment/index.html' | relative_url }}) — production lifecycle, starting with an existing Jenkins.
- [Prerequisites and compatibility]({{ '/reference-deployment/prerequisites.html' | relative_url }})
- [Acceptance checklist]({{ '/reference-deployment/acceptance-tests.html' | relative_url }})

## Integration

- [Connect an existing controller]({{ '/reference-deployment/integration/controller.html' | relative_url }})
- [Jenkins permissions]({{ '/reference-deployment/integration/permissions.html' | relative_url }})
- [Credentials]({{ '/reference-deployment/integration/credentials.html' | relative_url }})
- [Shared Libraries]({{ '/reference-deployment/integration/shared-libraries.html' | relative_url }})
- [Collection profiles]({{ '/reference-deployment/integration/collection.html' | relative_url }})
- [Knowledge Package]({{ '/reference-deployment/integration/package.html' | relative_url }})
- [Runtime CLI]({{ '/reference-deployment/integration/runtime.html' | relative_url }})
- [MCP server]({{ '/reference-deployment/integration/mcp.html' | relative_url }})
- [VS Code]({{ '/reference-deployment/integration/vscode.html' | relative_url }})

## Operations

- [Refresh]({{ '/reference-deployment/operations/refresh.html' | relative_url }})
- [Versioning and promotion]({{ '/reference-deployment/operations/versioning.html' | relative_url }})
- [Backup and restore]({{ '/reference-deployment/operations/backup-restore.html' | relative_url }})
- [Troubleshooting]({{ '/reference-deployment/operations/troubleshooting.html' | relative_url }})

## Optional demo

- [Installation]({{ '/reference-deployment/demo-environment/installation.html' | relative_url }})
- [Configuration]({{ '/reference-deployment/demo-environment/configuration.html' | relative_url }})
- [Test data]({{ '/reference-deployment/demo-environment/test-data.html' | relative_url }})
- [Walkthrough]({{ '/reference-deployment/demo-environment/walkthrough.html' | relative_url }})

## Technical reference

- [Architecture]({{ '/architecture.html' | relative_url }}) · [Catalog contract]({{ '/catalog-contract.html' | relative_url }})
- [Builder](https://github.com/dmji/jenkins-knowledge/blob/master/crates/knowledge-builder/README.md) · [MCP](https://github.com/dmji/jenkins-knowledge/blob/master/runtime/mcp.md) · [VS Code](https://github.com/dmji/jenkins-knowledge/blob/master/editors/vscode/README.md)

The production guide does not require Docker Compose or a new Jenkins installation. The demo is separate.

## Publishing

The documentation is built from `docs/` and deployed by the **Documentation Pages** workflow. In repository Settings → Pages, set the build and deployment source to **GitHub Actions**. Pull requests build the site without deploying it.
