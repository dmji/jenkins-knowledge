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
