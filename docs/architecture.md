# Application architecture and trust boundaries

## Operating model

Jenkins Knowledge is a two-stage application. Collection and user queries are separate workflows with different permissions, dependencies, and lifecycles.

```text
Administrator-controlled configuration
             |
             v
+---------------------------------------+
| Stage 1: Builder / collection         |
|                                       |
| - Configured Jenkins controllers      |
| - Configured Shared Library remotes   |
| - Admin-supplied credentials          |
| - Explicit quick/standard/deep run    |
| - Bounded collection and validation   |
+---------------------------------------+
             |
             v
      Versioned Knowledge Package
       (manifest + catalog + hashes)
             |
      explicit package selection
             |
             v
+---------------------------------------+
| Stage 2: Runtime / LSP / MCP          |
|                                       |
| - Opens one selected package          |
| - Local bounded queries               |
| - Read-only package access            |
| - No Jenkins/Git/network collection   |
| - No credentials or Builder           |
+---------------------------------------+
             |
             v
       IDE / MCP user queries
```

## Concrete component and executable map

The separation is expressed by the dependency graph, not by an arbitrary count of executables:

| Responsibility | Workspace crate / executable | Owns | Must not own |
|---|---|---|---|
| Package creation | **knowledge-builder** crate; **knowledge-builder** executable | Profile validation, Jenkins HTTP collection, configured Shared Library Git collection, version/plugin-aware snapshots, catalog assembly, package validation and publication | Runtime query APIs, LSP/MCP protocol handling |
| Shared Server core | **jenkins-runtime** crate | Opens and validates a selected package, builds local indexes, exposes read-only search/details/coverage/compatibility operations | Jenkins/Git connectivity, credentials, collection, package refresh |
| CLI query entry point | **jenkins-runtime** executable | Human-invoked offline queries against one selected package; JSON results on stdout and operational errors on stderr | Package collection or protocol serving |
| MCP adapter | **jenkins-mcp** executable in the **jenkins-runtime** crate | Line-delimited JSON-RPC over stdio; maps MCP tools/resources onto the shared Runtime API | Duplicated catalog/query logic, Jenkins/Git connectivity |
| LSP adapter | **jenkins-lsp** crate and executable | LSP framing, initialization/configuration, document context, completion, hover, definition, diagnostics and custom requests; delegates package access/querying to Runtime | Direct Builder/collector/scanner dependencies or Jenkins/Git connectivity |
| Offline stub generation | **groovy-stubs** executable in the **jenkins-runtime** crate | Reads the selected package and writes generated Groovy stubs to an explicit output file | Live source collection or protocol serving |
| Cross-component verification | **jenkins-contract-tests** crate | Test-only composition of Builder and Server; runs the actual LSP/MCP adapters as child processes against built packages | Production dependency of either Builder or Server |

Supporting libraries have narrow roles: **catalog-validator** is shared by package production and package consumption; **declarative-model**, **jenkins-gdsl-collector**, and **shared-library-scanner** are Builder-side inputs. The LSP depends on the Runtime crate; the MCP binary is compiled in the Runtime package and calls the same Runtime API directly. Both adapters therefore read the same catalog model and query implementation.

**jenkins-contract-tests** intentionally depends on both sides because its purpose is to verify the handoff. Its **contract-lsp** and **contract-mcp** helper binaries include the production adapter source and let integration tests exercise real stdio protocols. This test-only dependency does not reverse the production dependency direction.

## Interfaces and process contracts

- **Builder input:** an administrator-owned YAML profile, plus credentials supplied through the Builder process environment or the host's Git credential mechanisms. The profile determines the target controller, collection depth, configured libraries, and output package directory.
- **Builder output:** a validated Knowledge Package directory containing its manifest, catalog, schema/version metadata, provenance, collection outcomes, and integrity checksums. The package is data, not a platform executable.
- **Server input:** one explicit local package path. Runtime validates package metadata, schemas, references, and checksums before serving queries. Relative LSP package paths resolve against the workspace root; MCP and CLI accept an explicit package path (MCP also supports **JENKINS_KNOWLEDGE_PACKAGE**).
- **CLI behavior:** the Runtime CLI writes query JSON to stdout and errors to stderr. Builder writes build/plan summaries to stdout and failures to stderr. Neither CLI silently starts a long-running protocol server.
- **MCP behavior:** **jenkins-mcp** uses newline-delimited JSON-RPC on stdin/stdout. Only protocol responses are written to stdout; startup/validation failures go to stderr and terminate the process with a non-zero status.
- **LSP behavior:** **jenkins-lsp** uses the LSP stdio framing and initialization/configuration messages. Package selection is explicit; failed reloads unload the previous package rather than serving stale symbols.
- **Offline guarantee:** after package selection, Runtime, MCP, and LSP do not contact Jenkins, Git, or the network. The contract test starts a local listener as a tripwire and verifies that the package consumers make no request to it and do not mutate the package.

The stable boundary is the package contract plus the Runtime API. Builder may evolve its collection internals and the Server may expose additional protocol adapters or executables without changing who owns collection, package validation, and client-facing query behavior. The names and count of executables are packaging decisions, not architectural boundaries.

## Stage 1 — administrative collection

Builder is run explicitly by an administrator or an administrator-controlled automation pipeline. Its YAML profile is the allowlist of Jenkins controllers and Shared Library repositories to collect. Users of LSP/MCP cannot add a remote, choose an arbitrary Git URL, change the Builder profile, or initiate a scan.

Builder may access:
- the configured Jenkins HTTP(S) endpoints;
- the configured Shared Library Git remotes;
- credentials injected for that administrative run;
- its own temporary checkout and package staging directories.

Builder does not install plugins on Jenkins. Script Console collection, when enabled by the selected collection profile, uses only repository-owned fixed read-only probes. Collection is sequential/bounded according to the profile and does not run merely because a package is opened.

Git remotes are administrator-configured trusted inputs, not user-submitted hostile URLs. URL/ref validation, credential redaction, Git operation timeouts, shallow/no-tags fetch, isolated temporary checkouts, cleanup, and source/parser/output bounds remain required for correctness and operational resilience. The checkout-size threshold is a coarse sanity check at Git-operation boundaries; it is not a hard aggregate quota or a hostile-input security boundary. No elevated mount privilege, filesystem quota, or high-frequency filesystem polling is required.

A refresh is an explicit Builder run. It produces a new package/snapshot; it does not mutate a package currently served by Runtime.

## Bounded Jenkins API type inventory

The optional Script Console probe builds a bounded type graph from Pipeline step implementation classes, Global Variables, Declarative descriptors, and optional administrator-configured `apiRoots` for each controller. The roots are explicit inputs to the Builder collection run; Runtime, LSP, and MCP do not query Jenkins or trigger collection.

For selected roots, the probe records class/interface/enum identity, superclass and interfaces, public/protected method overloads, generic parameter and return types, inferred bean properties, plugin attribution, class-origin basenames, and descriptor help-file paths when exposed. It preserves unresolved, inaccessible, synthetic, bridge, and potentially dynamic metadata rather than treating incomplete reflection as complete API coverage.

The default hard limits are 20 configured roots, 40 types, traversal depth 2, 12 methods per type, 300 methods total, 20 parameters per method, 20 properties per type, and 256 KiB of probe output. Configured limits may be lowered but cannot exceed these bounds. Collection does not enumerate class loaders, scan packages or plugin JARs, or invoke discovered methods or constructors. Reflection metadata does not establish Script Security approval, CPS compatibility, or safe invocation.

This is a selected controller-observed API surface, not complete Groovy, Java, Jenkins, or plugin documentation. Unresolved and inaccessible information remains partial and must not be interpreted as evidence that an API is absent.

## Package boundary

The Knowledge Package is the handoff artifact between stages. It contains the manifest, catalog, schema/version metadata, provenance, collection outcomes, and integrity checksums. The catalog is the canonical data contract shared by Builder and all consumers.

Package publication is atomic from Builder's perspective: staging data is validated before publication. A package selected by Runtime is treated as immutable. To update knowledge, an administrator builds a new package and explicitly changes which package a Runtime process opens.

## Stage 2 — user queries

Runtime, the CLI query path, LSP, and MCP are package consumers. They:
- open one explicit package/profile;
- validate schemas, references, and checksums;
- perform bounded local search, lookup, completion, hover, diagnostics, and resource reads;
- preserve source identity, provenance, confidence, and incomplete-collection status.

They do not:
- contact Jenkins or Git;
- load or use Builder collection credentials;
- accept repository URLs or Jenkins endpoints as query inputs;
- invoke a collector, scanner, Script Console probe, or Builder;
- poll for remote changes or refresh a package implicitly;
- write, enrich, or append data to the selected package.

Read-only behavior is an architectural contract, not a UI convention. Deployments should mount the package read-only and give the Runtime process no collection credentials or unnecessary network access. Runtime still validates input sizes and bounds query work because a trusted package can grow accidentally or be malformed; these bounds are reliability controls, not a substitute for an untrusted-user Git sandbox.

## Failure and completeness semantics

Builder records which configured surfaces succeeded, were skipped, unavailable, denied, failed, or truncated. Optional source failures preserve successful independent data and mark the package partial. Runtime/LSP/MCP expose those outcomes without converting missing evidence into a verified absence or changing symbol confidence.

## Security and operational controls

Keep controls that protect the actual operating model:
- administrator-only source configuration and explicit collection;
- runtime-injected credentials, never serialized into artifacts or diagnostics;
- URL/ref validation and secret-safe errors;
- bounded Jenkins responses and sequential controller requests;
- Git operation timeouts and isolated temporary directories with reliable cleanup;
- bounded traversal depth, file count, per-file/aggregate source bytes, symbol count, catalog size, package loading, and query results;
- immutable package handoff and read-only Runtime permissions.

Do not add arbitrary-repository threat mitigations, platform-specific disk quotas, privileged mounts, or millisecond filesystem polling unless the product's trust model changes and the need is demonstrated.
