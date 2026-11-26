# Read-only MCP server

The jenkins-mcp binary exposes one validated Knowledge Package over the Model Context Protocol (MCP) stdio transport. It belongs to the user-query stage and reads the selected package only: it never connects to Jenkins or Git, starts collection, invokes Builder, accesses collection credentials, refreshes knowledge, executes Pipeline steps, or modifies package files.

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
| search_api | query?, kind?, source?, controller?, sourceKind?, context?, plugin?, owner?, type?, confidence?, offset?, limit? | Case-insensitive bounded search across Pipeline steps, globals, Declarative constructs, Groovy types/members, and Shared Library APIs |
| get_api_details | name_or_id | Exact symbol details; ambiguous names are errors |
| list_api | kind?, source?, controller?, sourceKind?, context?, plugin?, owner?, type?, confidence?, offset?, limit? | Bounded symbol listing |
| get_usage_examples | name_or_id | Catalog-backed examples and provenance |
| get_pipeline_step | name | Exact Jenkins Pipeline step details |
| get_controller_inventory | — | Controller identity, Core/plugin inventory, capability status, controller/package collection outcomes, profile, snapshot timestamps, and source provenance |

Valid confidence values are `verified`, `inferred`, and `unknown`. The default result limit is 50; callers may request 1–100 and use `offset` for bounded pagination. Results are sorted by name and then symbol ID. Controller, source kind, context, plugin, owner, and type filters use explicit catalog metadata and never infer missing values. Unknown or uninspected CPS, Sandbox, context, and plugin-availability data remains explicitly unknown in results.

`get_controller_inventory` and the matching resource return the selected package only. Plugin inventory status and capability status are independent of the presence of partial entries: an unknown or permission-limited inventory is not reported as complete. `freshness.controllerObservedAt` is the recorded controller snapshot time; `packageGeneratedAt` is the package generation time. Missing controller observation time is reported as unknown rather than inferred from package generation time.

## Resources

- jenkins://catalog/manifest — selected package manifest, including optional package collection completeness and per-source outcomes.
- jenkins://catalog/collection — normalized package completeness and per-source collection outcomes; older packages without this metadata report status `unknown`.
- jenkins://catalog/controller-inventory — combined controller identity, Jenkins Core and plugin sources/inventory, capability status, controller/package collection outcomes, profile, snapshot timestamps, and provenance.
- jenkins://catalog/libraries — pinned Shared Library source snapshots.
- jenkins://catalog/pipeline-steps — bounded Pipeline step symbols.
- jenkins://catalog/coverage — collection coverage records for the selected profile.
- jenkins://catalog/profiles/{profile} — selected profile metadata; only the loaded profile is served.

The server implements initialize, ping, tools/list, tools/call, resources/list, and resources/read over newline-delimited JSON-RPC stdio. It exposes no mutation tools or resource subscriptions.
