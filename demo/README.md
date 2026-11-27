# Local Jenkins Knowledge demo

This is an optional, disposable demonstration environment. It starts its own Jenkins 2.356 controller; it never connects to a production Jenkins instance and does not modify the Jenkins 2.356 compatibility test matrix.

## What it starts

- Jenkins 2.356-jdk17, pinned to the same linux/amd64 image digest used by the live integration tests.
- Builder and Runtime images based on a pinned, multi-architecture Ubuntu 24.04 manifest digest; their Rust binaries are built for the host architecture.
- The exact plugin dependency closure from tests/jenkins-2.356/plugins.lock.json, including SHA-256 verification.
- A local Jenkins jenkins-knowledge-demo Pipeline job.
- A Builder container that collects from the local controller using the read-only reader account and pins the GSA Shared Library to commit 7dfe2ab4bf7123b6aea29b0ec92caa2277cea5de.
- A read-only Runtime container, an MCP stdio service, and an LSP executable exposed to VS Code through demo/bin/jenkins-lsp.

The Jenkins service is published only on 127.0.0.1. It has explicit CPU/memory limits. Jenkins home and the Knowledge Package use separate named volumes. The generated admin/reader passwords are stored in the local, mode-0600 demo/.env; API tokens stay inside the Jenkins home volume. The Builder uses only the Overall/Read, Item/Read, and View/Read reader token. The admin token is used only to trigger the disposable demo Pipeline.

The demo needs outbound HTTPS to download the pinned Jenkins plugins and clone the public GSA Shared Library. It does not need access to any external Jenkins controller.

## Requirements

- Docker Engine with Docker Compose v2.20 or newer.
- Rust 1.100.0 (the repository-pinned toolchain), Cargo, Python 3, OpenSSL, and Git.
- Node.js 26.13.1/npm 11.20.0 for the VS Code extension-host acceptance test.
- VS Code for the interactive editor walkthrough.

The Jenkins image is pinned to linux/amd64. Docker Desktop on ARM hosts must support amd64 emulation; Builder and Runtime remain native to the host architecture.

## Start and build the package

Run from the repository root:

    bash demo/up.sh
    bash demo/build-package.sh

up.sh builds the release binaries and local images, resolves and verifies the pinned plugin archives, creates demo/.env with random demo-only passwords if needed, and waits for Jenkins readiness. build-package.sh waits for the token file, runs Builder inside its container against http://jenkins:8080, and starts Runtime only after the package has been published.

Jenkins is available at http://127.0.0.1:18080 by default. The generated admin and reader passwords are in demo/.env. The jenkins-knowledge-demo job runs a Pipeline against this local controller, loads the configured GSA Shared Library, and calls its example1 step at the guarded revision.

The Builder profile uses collectionProfile: standard; it does not invoke Script Console. Its Shared Library source and exact resolved revision are recorded in the package manifest and catalog. Startup and verification also check that the public repository's main ref still points to that pinned commit before the Jenkins Pipeline loads it.

## Verify the complete path

    bash demo/verify.sh

The verification script:

1. Runs the demo Pipeline and requires SUCCESS.
2. Queries the collected example1 Shared Library symbol through Runtime.
3. Performs MCP initialize and tools/list over stdio.
4. Launches a real VS Code extension host, with the LSP process executed inside the Runtime container. It verifies Shared Library completion, hover, revision-pinned go-to-definition, ordinary Groovy isolation, and profile mismatch/reload behavior.

The VS Code acceptance suite uses the same parameterized tests as CI; the demo supplies its actual package profile, symbol, source URL, and containerized LSP command. No Jenkins/Git collection occurs during Runtime, MCP, or LSP queries.

## Open the interactive VS Code demo

After up.sh and build-package.sh, compile the extension once:

    npm --prefix editors/vscode ci
    npm --prefix editors/vscode run compile

Then open the demo workspace:

    code --extensionDevelopmentPath="$PWD/editors/vscode" "$PWD/demo/jenkins-knowledge.code-workspace"

The workspace file selects /package/knowledge-package inside the Runtime container and starts demo/bin/jenkins-lsp, which forwards the VS Code language-server stdio stream into that container. Open demo/Jenkinsfile.demo and try example1() to see the pinned Shared Library completion, hover details, and definition link. The Jenkins job created at startup is a separate inline Pipeline smoke test.

For a standalone MCP client, configure its stdio command as:

    docker compose --profile mcp --project-directory "$PWD/demo" \
      --env-file "$PWD/demo/.env" \
      -f "$PWD/demo/compose.yml" run --rm -T mcp

## Stop or reset

    bash demo/down.sh

This stops containers but preserves Jenkins home, the package volume, and the generated credentials so the demo can be resumed with bash demo/up.sh.

To remove all demo data and generated credentials:

    bash demo/down.sh --reset

The reset removes the Compose volumes, local demo images, generated plugin build context, and demo/.env. It does not remove unrelated repository target/ contents.

Do not commit demo/.env or reuse these demo credentials outside this disposable local environment.
