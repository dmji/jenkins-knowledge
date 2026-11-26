# Jenkins Knowledge for VS Code

This is a thin VS Code client for the Rust language server in `crates/jenkins-lsp`. It coexists with installed Groovy tooling; ordinary Groovy parsing and language features remain the responsibility of that tooling. Jenkins catalog completions, hover, navigation, and diagnostics are restricted to Jenkinsfile documents.

## Build

From the repository root:

```sh
cargo build --release --manifest-path crates/jenkins-lsp/Cargo.toml
cd editors/vscode
npm ci
npm run compile
```

## Configure

Set the following workspace settings:

```json
{
  "jenkinsKnowledge.serverPath": "target/release/jenkins-lsp",
  "jenkinsKnowledge.packagePath": "target/fixture-package",
  "jenkinsKnowledge.profile": "fixture",
  "files.associations": {
    "Jenkinsfile": "groovy",
    "*.jenkinsfile": "groovy"
  }
}
```

Build the package before starting VS Code:

```sh
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- build profiles/fixture.yml target/fixture-package
```

The package path is resolved relative to the workspace. The Rust server validates the package using Runtime and rejects a configured profile mismatch. Changing `jenkinsKnowledge.packagePath` or `jenkinsKnowledge.profile` reloads a different local package selection; it does not run Builder, contact Jenkins/Git, or refresh the package contents. Use **Jenkins Knowledge: Reload Knowledge Package** to explicitly reload the selected local package/profile. Invalid configuration clears catalog results and diagnostics rather than retaining stale symbols.

## Extension-host acceptance

The reproducible acceptance suite launches a pinned VS Code 1.85.0 extension host with the real `jenkins-lsp` binary. It consumes the Deep package produced by the live Jenkins 2.356 integration job plus deterministic primary/alternate fixture packages.

From the repository root, after `tests/jenkins-2.356/run.sh` has completed:

```sh
bash tests/jenkins-2.356/run-vscode-acceptance.sh
```

The suite verifies GDSL completion and a revision-pinned Shared Library definition against the live package; positive hover/navigation and an explicit unavailable-plugin diagnostic against the primary fixture; comment/string/unknown-name negative cases; package reload/profile isolation against the alternate fixture; and no Jenkins-specific completion in ordinary Groovy files. It runs without a public Jenkins service or credentials. The integration workflow runs it automatically after the live Jenkins matrix.

## Features and limitations

- Scripted and Declarative Jenkinsfiles receive context-aware completion for registered Pipeline steps, globals, Declarative extensions, Shared Library symbols, and selected Jenkins core/plugin types.
- Hover includes overloads, parameters, return types, plugin availability, source revision/provenance, CPS/Sandbox constraints, confidence, and profile.
- Go-to-definition opens a revision-pinned GitHub source location when the catalog contains a safe file path.
- Diagnostics report only verified, explicit catalog evidence: invalid Declarative contexts, unavailable plugin symbols, CPS incompatibility, and sandbox rejection. Unknown or uninspected values are not errors.
- Ordinary Groovy source files continue to use the installed Groovy tooling without Jenkins catalog completions.
- Results come from one explicitly selected local Knowledge Package; symbols from other profiles are never mixed in.

## Smoke test

1. Build the fixture Knowledge Package and the Rust server.
2. Open a Groovy workspace containing a Jenkinsfile.
3. Configure the package path and profile as above, then reload the VS Code window.
4. In a Scripted Pipeline (`node { ... }`), type `sh`, a controller/plugin global, or a Shared Library global such as `deployApp`; verify completion appears. In a Declarative Pipeline, verify directives appear at pipeline/stage level and disappear inside `steps`.
5. Hover an overloaded plugin method; verify each signature, parameter/return details, plugin/source revision, availability, provenance, constraints, confidence, and profile are shown.
6. Use Go to Definition on a Shared Library symbol with a recorded file path; verify the link opens the matching revision. Symbols without safe source locations should have no definition target.
7. Add a verified unavailable-plugin symbol or known CPS/Sandbox restriction to a fixture Jenkinsfile; verify a diagnostic includes source evidence. Remove the evidence or set it to `unknown`; verify the diagnostic disappears. Comments and strings must not trigger diagnostics.
8. Change the package path/profile or run **Jenkins Knowledge: Reload Knowledge Package**; verify completions, hover, and diagnostics switch together. Select a mismatched profile and verify old results and diagnostics are cleared.
9. Open an ordinary Groovy source file; verify Jenkins catalog completions do not appear and the installed Groovy tooling remains active.

The extension host is intentionally limited to process startup and configuration forwarding. Catalog access and language intelligence remain in Rust.
