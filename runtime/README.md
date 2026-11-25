# Runtime

The Runtime is the shared, read-only source of truth for loading and querying a validated Knowledge Package. CLI, MCP, and LSP use the same symbol enrichment, provenance, confidence, filtering, and bounded search implementation. It performs no Jenkins, Git, or network access.

## Query contract

```rust
SearchOptions {
    query,
    kind,
    source,
    plugin,
    owner,
    type_name,
    confidence,
    limit,
}
```

All filters are optional. Text matching is case-insensitive across name, description, signature, namespace, and return type. Plugin matches the symbol's plugin source or declared plugin dependencies; owner matches explicit owner/namespace/class/type metadata; type matches explicit type/returnType/namespace/class metadata. Missing metadata does not produce inferred matches.

Results are sorted by symbol name and then stable symbol ID. The default limit is 50 and the hard maximum is 100. A caller cannot request an unbounded result set. Exact detail lookups reject ambiguous names and direct callers to use a symbol ID.

The Runtime also exposes catalog coverage without converting missing or unavailable coverage into verified symbols.

## CLI

```sh
cargo run -p jenkins-runtime -- --package target/fixture-package search deploy --kind global-variable --confidence inferred --limit 20
cargo run -p jenkins-runtime -- --package target/fixture-package list --plugin workflow-basic-steps
```

Supported search filters: `--kind`, `--source`, `--plugin`, `--owner`, `--type`, `--confidence`, and `--limit`. Valid confidence values are `verified`, `inferred`, and `unknown`. Search and list now return an envelope containing `profile`, `limit`, and `results`; consumers that previously expected a bare array should read the `results` field.

## LSP

The Jenkins LSP uses Runtime search for completion and hover, preserving the same deterministic ordering and maximum result bound. It exposes `jenkinsKnowledge/search` with the shared filters and `jenkinsKnowledge/coverage` as read-only custom requests.

## MCP

See [MCP server documentation](mcp.md) for tool and resource schemas. MCP `search_api` and `list_api` expose the same filters and limits as the Runtime API, and `jenkins://catalog/coverage` returns coverage for the loaded profile.
