# Configure the VS Code extension

The repository's VS Code extension is a thin client for the Rust **jenkins-lsp** executable. It uses the Language Server Protocol over stdio and reads a selected local Knowledge Package through Runtime. It does not start Builder or connect to Jenkins/Git.

Build the LSP and extension from the repository root:

~~~sh
cargo build --release --locked --manifest-path crates/jenkins-lsp/Cargo.toml
cd editors/vscode
npm ci
npm run compile
~~~

Configure the workspace:

~~~json
{
  "jenkinsKnowledge.serverPath": "/opt/jenkins-knowledge/bin/jenkins-lsp",
  "jenkinsKnowledge.packagePath": "/srv/jenkins-knowledge/production-pipelines",
  "jenkinsKnowledge.profile": "production-pipelines",
  "files.associations": {
    "Jenkinsfile": "groovy",
    "Jenkinsfile.*": "groovy",
    "*.jenkinsfile": "groovy"
  }
}
~~~

The package path can be absolute or relative to the workspace root. The optional profile must match the package manifest. The extension passes these settings to the LSP during initialization and synchronizes later configuration changes. Use **Jenkins Knowledge: Reload Knowledge Package** after publishing a new package. A failed reload clears old catalog results and diagnostics rather than retaining stale symbols.

## What users see

- **Completion:** Scripted Pipeline steps, globals, Shared Library symbols, and context-supported Core/plugin types. Declarative directives appear outside steps; script blocks use Scripted/Groovy scope.
- **Hover:** exact-name overloads, parameter/return details, plugin availability, source revision/provenance, execution constraints, confidence, and selected profile.
- **Go to Definition:** a revision-pinned GitHub source link only when safe source-path evidence exists. No source link is invented when provenance is incomplete.
- **Diagnostics:** verified invalid Declarative contexts, unavailable plugin symbols, CPS incompatibility, and Script Security Sandbox rejection. Unknown or uninspected evidence is not reported as an error.

Jenkins catalog features are limited to Jenkinsfile, Jenkinsfile.* and *.jenkinsfile documents. Ordinary .groovy files continue to use installed Groovy tooling. This is a bounded catalog-backed assistant, not a complete Groovy parser or Pipeline execution validator.

For the full feature reference, direct LSP initialization/configuration contract, custom requests, and LSP-versus-MCP comparison, see the [LSP and MCP interface guide]({{ '/interfaces.html' | relative_url }}). For the extension-host acceptance procedure, see the [VS Code extension README](https://github.com/dmji/jenkins-knowledge/blob/master/editors/vscode/README.md).
