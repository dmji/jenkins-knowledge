# Jenkins Knowledge for VS Code

This is a thin VS Code client for the Rust language server in `crates/jenkins-lsp`. It registers only for Groovy documents and coexists with the installed Groovy language tooling; ordinary Groovy parsing and language features remain the responsibility of that tooling.

## Build

From the repository root:

```sh
cargo build --release --manifest-path crates/jenkins-lsp/Cargo.toml
cd editors/vscode
npm install
npm run compile
```

## Configure

Set the following workspace settings:

```json
{
  "jenkinsKnowledge.serverPath": "${workspaceFolder}/target/release/jenkins-lsp",
  "jenkinsKnowledge.packagePath": "target/fixture-package",
  "jenkinsKnowledge.profile": "fixture"
}
```

Build the package before starting VS Code:

```sh
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- build profiles/fixture.yml target/fixture-package
```

The package path is resolved relative to the workspace. The Rust server validates the package using Runtime and rejects a configured profile mismatch. Changing `jenkinsKnowledge.packagePath` or `jenkinsKnowledge.profile` reloads the selected package; invalid configuration clears catalog results rather than retaining stale symbols.

## Features and limitations

- Completion includes catalog-backed Pipeline steps and Shared Library globals, methods, classes, and properties.
- Hover shows catalog signature, description, source, confidence, and selected profile.
- Results come from one explicitly selected Knowledge Package; symbols from other profiles are never mixed in.
- This is additive Jenkins catalog intelligence, not a replacement for a Groovy language server. It does not validate Jenkinsfile semantics or execute Pipeline code.

## Smoke test

1. Build the fixture Knowledge Package and the Rust server.
2. Open a Groovy workspace containing a Jenkinsfile.
3. Configure the package path and profile as above, then reload the VS Code window.
4. Type `sh` or a catalog-backed Shared Library global such as `deployApp`; verify completion appears.
5. Hover over a catalog-backed symbol; verify signature, description, source, confidence, and profile are shown.
6. Change the package path to another package with a different profile and verify the completion/hover results switch. Configure a mismatched profile and verify the server reports the mismatch and returns no catalog results.

The extension host is intentionally limited to process startup and configuration forwarding. Catalog access and language intelligence remain in Rust.
