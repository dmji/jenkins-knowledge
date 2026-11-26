# Jenkins Knowledge Runtime

Runtime opens a checksummed Knowledge Package, validates its manifest and catalog, and exposes bounded catalog queries.

## Package loading limits

The loader rejects oversized inputs before parsing or retaining their contents. Defaults are fixed to keep package loading bounded:

| Limit | Default |
|---|---:|
| Manifest JSON | 1 MiB |
| Checksum entries | 10,000 |
| Each checksummed package file | 64 MiB |
| Aggregate manifest and checksummed files | 256 MiB |
| Catalog sources | 10,000 |
| Catalog symbols | 100,000 |

The manifest and each checksummed file are read through a bounded reader. Every declared file is checksum-verified, but only the catalog bytes are retained after verification; other file buffers are released before the next file is read. Package paths must remain regular files inside the package directory and cannot traverse symlinks.

Limits are intentionally not user-configurable: changing them changes the maximum resource exposure of every Runtime consumer. Exceeded limits return explicit errors naming the violated ceiling.

## Search complexity

Runtime builds a source-ID-to-source-position index once when opening the package. Search uses that index for plugin matching and result enrichment rather than scanning the source array for each symbol.

Search still inspects catalog symbols to evaluate filters, but it maintains only the best requested results in a bounded heap. For N symbols and result limit K (1–100), selection takes O(N log K) time and O(K) result-selection memory, followed by O(K log K) ordering. Results are deterministic, ordered by symbol name and then symbol ID. This avoids collecting and sorting every match when callers request a small result page.


## Large-catalog consumers

LSP completion uses the identifier prefix at the cursor and the open document's Pipeline context. Declarative directives are suggested in Declarative Pipeline blocks, Pipeline steps in steps/scripted contexts, and Shared Library symbols are queried separately by source kind. The server returns an LSP CompletionList with isIncomplete set when more than 100 relevant items match, allowing clients to request again as the prefix becomes more specific.

The MCP \`search_api\` and \`list_api\` tools use Runtime's bounded \`search_page\` API. Both accept \`offset\` (default 0, range 0–10,000) and \`limit\` (default 50, range 1–100) in addition to their existing filters. Responses retain the existing \`profile\`, \`results\`, and \`limit\` fields and add \`offset\`, \`total\`, \`returned\`, \`hasMore\`, and \`nextOffset\`. \`nextOffset\` is null on the final page; \`paginationLimitReached\` indicates that more matches exist beyond the Runtime offset ceiling and cannot be requested through this API. Invalid types, negative values, and out-of-range values return actionable tool errors rather than silently reverting to defaults.

Ordering is deterministic by symbol name and then symbol ID. Runtime selects each page with bounded O(K) result-selection memory (K is the requested page size); clients can traverse pages without collecting the full catalog in one response.

The MCP resource \`jenkins://catalog/pipeline-steps\` remains valid and returns its first page using the default limit of 50. Clients can request further pages with \`jenkins://catalog/pipeline-steps?offset=50&limit=50\`. It uses the same pagination metadata and offset/limit bounds. Unknown, duplicate, malformed, or out-of-range query parameters are rejected.
