# Reference deployment and operations

This guide starts with an **existing Jenkins controller** already operated by your organization. Jenkins installation and administration are outside the required Jenkins Knowledge lifecycle.

## Lifecycle

1. [Check prerequisites and compatibility](prerequisites.md).
2. [Connect the controller](integration/controller.md).
3. Create a [least-privilege service account](integration/permissions.md) and [provide credentials securely](integration/credentials.md).
4. [Configure Shared Libraries](integration/shared-libraries.md), if needed.
5. [Choose a profile and collect](integration/collection.md).
6. [Inspect package completeness and publish](integration/package.md).
7. Distribute the package to consumer hosts.
8. Configure [Runtime](integration/runtime.md), [MCP](integration/mcp.md), and [VS Code](integration/vscode.md).
9. Operate releases using [refresh, versioning, backup, and troubleshooting](operations/refresh.md).

## Trust boundary

Builder is an administrator-operated, explicitly invoked collection process. Runtime/LSP/MCP consume one already-built package offline and read-only. Keep collection credentials away from consumer hosts, and mount package files read-only.

## Optional demonstration

The [Demo Environment](demo-environment/installation.md) starts a disposable Jenkins 2.356 / Java 17 controller. It is not a prerequisite or production deployment recipe.
