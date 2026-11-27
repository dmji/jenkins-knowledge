# Configure the MCP server

`jenkins-mcp` exposes one validated Knowledge Package over MCP stdio. One server process serves exactly one package/profile.

```sh
cargo build --locked --manifest-path crates/jenkins-runtime/Cargo.toml --bin jenkins-mcp
cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml --bin jenkins-mcp -- --package /srv/jenkins-knowledge/production-pipelines
```

Example client configuration (adjust absolute paths):

```json
{
  "mcpServers": {
    "jenkins-knowledge": {
      "command": "/opt/jenkins-knowledge/bin/jenkins-mcp",
      "args": ["--package", "/srv/jenkins-knowledge/production-pipelines"]
    }
  }
}
```

Read-only tools cover search/list, exact symbol details, examples, Pipeline steps, controller inventory, and offline compatibility reporting. Resources expose manifest, collection, inventory, compatibility, libraries, Pipeline steps, coverage, and profile metadata. Search/list default to 50; callers may request 1–100 and use offset pagination. The server does not execute Pipeline steps or connect to Jenkins/Git. See [full MCP contract](https://github.com/dmji/jenkins-knowledge/blob/master/runtime/mcp.md).
