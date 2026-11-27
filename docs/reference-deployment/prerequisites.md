# Prerequisites and compatibility

## Existing Jenkins

Builder needs HTTP(S) reachability to an existing controller. The current catalog supports **one controller snapshot per package**. Record the actual Core version and plugin inventory; do not infer Java version from Jenkins version. Java version/vendor are observed only by the opt-in Deep Script Console probe. Quick and Standard leave them not collected.

## Builder host

- An OS supported by the pinned Rust toolchain and Git client.
- Rust **1.100.0** to build from source (see `rust-toolchain.toml`).
- Git for Shared Library collection.
- Network access to configured Jenkins and Git sources only.
- Restricted writable output/cache directories.

Node.js 26.13.1/npm 11.20.0 are needed to build/test the VS Code extension. Docker is needed only for container images and the optional demo, not for ordinary collection or package consumers.

## Consumer hosts

Runtime/MCP/LSP need the selected package and executable; they do not need Jenkins/Git credentials, Docker, or controller network access. VS Code additionally requires VS Code and the built `jenkins-lsp` binary.

## Evidence-based compatibility

Compatibility depends on actual Core/plugin versions, permissions, GDSL availability, Shared Library revisions, and collection profile. The live CI target is Jenkins 2.356 with Java 17 and a pinned plugin set; this is a test target, not a minimum or recommended production version. See [the pinned matrix](https://github.com/dmji/jenkins-knowledge/blob/master/tests/jenkins-2.356/README.md).
