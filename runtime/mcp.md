# Read-only MCP server

The jenkins-mcp binary exposes one validated Knowledge Package over the Model Context Protocol (MCP) stdio transport. It reads local package files only: it never connects to Jenkins, executes Pipeline steps, accesses credentials, or writes to a controller.

## Build and launch

Run from the repository root so the JSON schemas are available:

```sh
cargo build --manifest-path crates/jenkins-runtime/Cargo.toml --bin jenkins-mcp
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml --bin jenkins-mcp -- --package ./target/fixture-package
```

A package is mandatory. JENKINS_KNOWLEDGE_PACKAGE may be used instead. One server process serves exactly one package/profile. To switch profiles, launch another process with that profile's package; results never mix library versions.

## MCP client configuration

```json
{
  "mcpServers": {
    "jenkins-knowledge": {
      "command": "cargo",
      "args": ["run", "--quiet", "--manifest-path", "/path/to/jenkins-knowledge/crates/jenkins-runtime/Cargo.toml", "--bin", "jenkins-mcp", "--", "--package", "/path/to/knowledge-package"]
    }
  }
}
```

For a prebuilt binary, use its absolute path as command and remove the Cargo arguments.

## Tools

All tools accept JSON object arguments and return catalog data as MCP text content. Search and list share the Runtime query contract; results preserve profile, source snapshot, confidence, and provenance.

| Tool | Input | Behavior |
| --- | --- | --- |
| search_api | query?, kind?, source?, plugin?, owner?, type?, confidence?, limit? | Case-insensitive bounded catalog search |
| get_api_details | name_or_id | Exact symbol details; ambiguous names are errors |
| list_api | kind?, source?, plugin?, owner?, type?, confidence?, limit? | Bounded symbol listing |
| get_usage_examples | name_or_id | Catalog-backed examples and provenance |
| get_pipeline_step | name | Exact Jenkins Pipeline step details |

Valid confidence values are `verified`, `inferred`, and `unknown`. The default result limit is 50; callers may request 1–100. Results are sorted by name and then symbol ID. Plugin, owner, and type filters use explicit catalog metadata and never infer missing values.

## Resources

- jenkins://catalog/manifest — selected package manifest.
- jenkins://catalog/libraries — pinned Shared Library source snapshots.
- jenkins://catalog/pipeline-steps — bounded Pipeline step symbols.
- jenkins://catalog/coverage — collection coverage records for the selected profile.
- jenkins://catalog/profiles/{profile} — selected profile metadata; only the loaded profile is served.

The server implements initialize, ping, tools/list, tools/call, resources/list, and resources/read over newline-delimited JSON-RPC stdio. It exposes no mutation tools or resource subscriptions.
