# Deployment reference

The [end-to-end deployment guide]({{ '/reference-deployment/guide.html' | relative_url }}) is the recommended starting point. It explains the complete path from an existing Jenkins controller to a local developer workflow without requiring the reader to assemble the process from separate pages.

This section retains focused references for readers who need a specific detail or want to verify one part of an existing deployment.

## Before collection

- [Prerequisites and compatibility]({{ '/reference-deployment/prerequisites.html' | relative_url }}) — toolchain, network, and compatibility evidence.
- [Connect a controller]({{ '/reference-deployment/integration/controller.html' | relative_url }}) — profile shape and endpoint constraints.
- [Permissions]({{ '/reference-deployment/integration/permissions.html' | relative_url }}) and [credentials]({{ '/reference-deployment/integration/credentials.html' | relative_url }}) — least privilege and secret handling.
- [Shared Libraries]({{ '/reference-deployment/integration/shared-libraries.html' | relative_url }}) — pinned revisions and scanner boundaries.

## Collection and consumers

- [Collection profiles and commands]({{ '/reference-deployment/integration/collection.html' | relative_url }})
- [Package structure and completeness]({{ '/reference-deployment/integration/package.html' | relative_url }})
- [Runtime CLI]({{ '/reference-deployment/integration/runtime.html' | relative_url }})
- [MCP server]({{ '/reference-deployment/integration/mcp.html' | relative_url }})
- [VS Code extension]({{ '/reference-deployment/integration/vscode.html' | relative_url }})

## Operations

- [Refresh]({{ '/reference-deployment/operations/refresh.html' | relative_url }}) — explicit rebuild workflow and cache behavior.
- [Versioning and promotion]({{ '/reference-deployment/operations/versioning.html' | relative_url }}) — immutable releases and rollback.
- [Backup and restore]({{ '/reference-deployment/operations/backup-restore.html' | relative_url }})
- [Troubleshooting]({{ '/reference-deployment/operations/troubleshooting.html' | relative_url }})
- [Acceptance checklist]({{ '/reference-deployment/acceptance-tests.html' | relative_url }})

## Optional demo

The [Demo Environment]({{ '/reference-deployment/demo-environment/installation.html' | relative_url }}) is a disposable Jenkins 2.356 / Java 17 test fixture. It is not required for production integration and should not be mistaken for a production deployment recipe.

---

[Back to documentation home]({{ '/index.html' | relative_url }})
