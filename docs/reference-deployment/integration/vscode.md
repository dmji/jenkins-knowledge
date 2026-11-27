# Configure the VS Code extension

The extension is a thin client for Rust `jenkins-lsp`. Jenkins catalog completion, hover, navigation, and evidence-based diagnostics are restricted to Jenkinsfile documents; ordinary Groovy features remain with installed Groovy tooling.

```sh
cargo build --release --locked --manifest-path crates/jenkins-lsp/Cargo.toml
cd editors/vscode
npm ci
npm run compile
```

Workspace settings:

```json
{
  "jenkinsKnowledge.serverPath": "/opt/jenkins-knowledge/bin/jenkins-lsp",
  "jenkinsKnowledge.packagePath": "/srv/jenkins-knowledge/production-pipelines",
  "jenkinsKnowledge.profile": "production-pipelines",
  "files.associations": {
    "Jenkinsfile": "groovy",
    "*.jenkinsfile": "groovy"
  }
}
```

Build/validate the package first. The profile must match the manifest. Use **Jenkins Knowledge: Reload Knowledge Package** after publishing a new package; changing settings does not invoke Builder or refresh sources. LSP/extension need local package access, not Jenkins/Git credentials. See [extension documentation](../../../../editors/vscode/README.md).
