# Reference deployment and operations

This guide starts with an **existing Jenkins controller** already operated by your organization. Jenkins installation and administration are outside the required Jenkins Knowledge lifecycle.

## Lifecycle

1. [Check prerequisites and compatibility]({{ '/reference-deployment/prerequisites.html' | relative_url }}).
2. [Connect the controller]({{ '/reference-deployment/integration/controller.html' | relative_url }}).
3. Create a [least-privilege service account]({{ '/reference-deployment/integration/permissions.html' | relative_url }}) and [provide credentials securely]({{ '/reference-deployment/integration/credentials.html' | relative_url }}).
4. [Configure Shared Libraries]({{ '/reference-deployment/integration/shared-libraries.html' | relative_url }}), if needed.
5. [Choose a profile and collect]({{ '/reference-deployment/integration/collection.html' | relative_url }}).
6. [Inspect package completeness and publish]({{ '/reference-deployment/integration/package.html' | relative_url }}).
7. Distribute the package to consumer hosts.
8. Configure [Runtime]({{ '/reference-deployment/integration/runtime.html' | relative_url }}), [MCP]({{ '/reference-deployment/integration/mcp.html' | relative_url }}), and [VS Code]({{ '/reference-deployment/integration/vscode.html' | relative_url }}).
9. Operate releases using [refresh]({{ '/reference-deployment/operations/refresh.html' | relative_url }}), [versioning]({{ '/reference-deployment/operations/versioning.html' | relative_url }}), [backup/restore]({{ '/reference-deployment/operations/backup-restore.html' | relative_url }}), and [troubleshooting]({{ '/reference-deployment/operations/troubleshooting.html' | relative_url }}).

## Trust boundary

Builder is an administrator-operated, explicitly invoked collection process. Runtime/LSP/MCP consume one already-built package offline and read-only. Keep collection credentials away from consumer hosts, and mount package files read-only.

## Optional demonstration

The [Demo Environment]({{ '/reference-deployment/demo-environment/installation.html' | relative_url }}) starts a disposable Jenkins 2.356 / Java 17 controller. It is not a prerequisite or production deployment recipe.

[Back to documentation home]({{ '/index.html' | relative_url }})
