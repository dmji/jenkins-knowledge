# Catalog contract

The catalog is the canonical API representation shared by Builder, Runtime, IDE tooling, and MCP. The JSON Schema in `schemas/catalog.schema.json` is normative; generated stubs and downstream indexes must not add API facts absent from the catalog.

## Top-level fields

- `schemaVersion`: catalog schema version. Version 1 is `1.0.0`.
- `catalogId`: stable identifier for this catalog snapshot.
- `generatedAt`: RFC 3339 timestamp.
- `snapshotId` (optional for legacy catalogs): stable SHA-256 identity for the logical snapshot.
- `completeness` (optional for legacy catalogs): package-level `complete`, `partial`, or `unknown` outcome.
- `sources`: source snapshots from which symbols were extracted.
- `symbols`: API symbols associated with exactly one source.
- `coverage` (optional): per-source collection status for an inspected API surface.

## Sources

Each source has a unique `id`, a `kind`, a human-readable `name`, and a `revision`. Shared Library revisions must identify the resolved Git commit SHA; the configured branch or tag belongs in `version` or source metadata. Controller revisions identify the Jenkins version/snapshot used for extraction. Optional URLs and metadata provide traceability but must not contain credentials.

## Controller runtime snapshot

The optional top-level `controller` object describes one concrete Jenkins controller. It is additive to schema 1.0.0: existing GDSL-only and Shared Library catalogs remain valid when it is absent.

- `identity` contains a stable controller identifier and optional display name/base URL. URLs must be credential-free.
- `coreVersion` records the observed Jenkins Core version.
- `plugins` is a bounded inventory of installed plugins. Each entry references a `plugin` source and records the short name, observed version, enabled/active state (nullable when unknown), and declared dependencies. A missing plugin is not implied by an incomplete inventory.
- `capabilities` records named probes as `available`, `unavailable`, or `unknown`; it does not infer capability from a Core version.
- `collection` records the profile, retrieval timestamp, optional start/completion timestamps, and request count.

A symbol may include `availability` and `executionConstraints`. Availability is explicitly `available`, `unavailable`, or `unknown`, with optional plugin source references. CPS and Script Security/Sandbox values distinguish positive/negative observations from `unknown` and `not-inspected`. These fields must only be populated from evidence; the presence of a method or step does not prove CPS compatibility or Sandbox approval.

The optional `evidence` array attaches source identity, evidence class (`controller-observed`, `source-scanned`, `static-model`, or `inferred`), confidence, retrieval time, and an optional evidence location to an individual symbol. Evidence source IDs and plugin availability references must resolve to catalog sources. Existing `sourceId`, `provenance`, `confidence`, and `fieldEvidence` remain supported for older producers.

## Evidence and unknown values

Use `null`, `unknown`, or `not-inspected` according to the field's schema; do not replace missing observations with version-based guesses. Partial collection is valid: record the profile, retrieval time, known facts, and explicit capability/coverage status. A failed or forbidden probe is not proof that a feature is absent. Controller snapshots must not include credentials, tokens, authorization headers, or secret-bearing metadata.

## Symbols

Each symbol has a unique `id`, a non-empty `name`, a `kind`, a `sourceId` referencing a catalog source, and a `confidence`. Optional fields describe namespace, signature, description, parameters, return type, overloads, examples, constraints, contexts, documentation URL, provenance, and per-field `fieldEvidence`.

Symbol kinds:

- `step`: executable Pipeline step, commonly contributed by Jenkins core or a plugin.
- `global-variable`: a Shared Library global exposed through `vars/`.
- `method`: callable method, including methods discovered in library source.
- `class`: a type exposed by a library or Jenkins API.
- `property`: a readable/writable property on a type or global.
- `declarative-directive`: Declarative Pipeline section or directive, not an executable step.

Parameters carry a name and may include type, requiredness, description, and a JSON-compatible default value. Signatures and overloads are descriptive source facts; consumers must not infer missing parameter types or defaults.

## Confidence

- `verified`: directly supported by authoritative extracted evidence such as controller GDSL or source declarations.
- `inferred`: derived heuristically (for example, dynamic Groovy analysis) and not guaranteed by an explicit declaration.
- `unknown`: the symbol is known to exist, but one or more relevant details cannot be established reliably.

Confidence applies to the symbol as a whole. Missing facts should remain absent rather than being fabricated.

## Provenance

The `provenance` array records human-readable evidence locations or extraction notes. Source identity and revision are carried by `sourceId` and the referenced source object. A symbol whose source cannot be established must not be emitted as a normal catalog symbol.

## Evidence quality and collection coverage

Three independent dimensions must not be conflated:

- **Confidence** describes the overall strength of the symbol claim (`verified`, `inferred`, or `unknown`).
- **Provenance** describes where the symbol evidence came from. The source ID and revision identify the source snapshot; the optional `provenance` strings record evidence locations or extraction notes.
- **Coverage** describes what collection actually inspected. The optional top-level `coverage` array records a `sourceId`, a `surface`, and one status: `inspected`, `succeeded`, `succeeded-with-truncation`, `unavailable`, `permission-denied`, `failed`, `skipped-by-profile`, `not-configured`, `unsupported`, or `not-collected`. Coverage entries must reference an existing source. Absence of a coverage entry is not evidence that a surface was inspected.

The optional symbol `fieldEvidence` object records evidence quality independently for individual fields, using the same `verified`, `inferred`, and `unknown` values. A missing field-evidence entry means unknown; symbol-level `confidence: verified` must not be interpreted as proof that every optional attribute is verified. Producers should only mark a field verified when its value was directly observed or declared by the referenced source.

## Validation and compatibility

Validation applies JSON Schema constraints and semantic rules: unique source IDs and symbol IDs, valid symbol and coverage source references, one coverage record per source/surface pair, unique parameter names within a symbol, and rejection of credential-bearing source URLs or secret-named fields. Duplicate source identities are rejected during package merge rather than overwritten. Same-name symbols from distinct sources remain distinct when their IDs and source references are distinct. Conflicts are reported with both contributing input labels and revisions.

Unknown properties are rejected. The optional `coverage` and `fieldEvidence` fields are additive and do not invalidate existing 1.0.0 catalogs. Changes to required fields, enum values, or field meaning require an explicit schema version decision and migration notes. The current schema rejects unsupported schema versions rather than guessing.

## Snapshot identity and source collection outcomes

`catalogId` identifies the logical catalog namespace; it is not a collection-run identifier. `snapshotId` is a SHA-256 digest (`sha256:` prefix) of the canonical logical catalog. The digest is independent of JSON object-key insertion order and excludes collection timestamps (`generatedAt`, `createdAt`, `startedAt`, `completedAt`, `retrievedAt`, and `collectedAt`). Identity-bearing arrays are normalized by stable IDs or names before hashing. Any other content change changes the snapshot ID.

Each source `id` identifies the logical configured source within a package and must remain stable as that source advances. `revision` identifies the source-native revision: a Shared Library uses the resolved Git commit; Jenkins controller discovery uses a digest of the observed controller snapshot; GDSL uses a digest of the retrieved DSL document; static models use their model version. Collection timestamps are never source revisions. `fingerprint` is the SHA-256 digest of the source record, its symbols, its coverage entries, and the associated controller/plugin observation after volatile timestamps are removed.

Symbol IDs are deterministic within a source: source ID + symbol kind + namespace + name + normalized signature. Line numbers and retrieval timestamps belong in provenance, not identity. Source IDs remain stable for the same logical configured repository/controller and reference; a moving branch does not change source identity, while its resolved commit changes the source revision. A symbol that cites a secondary evidence source contributes to both the primary source fingerprint and the referenced evidence-source fingerprint. Different sources may expose the same name or signature; their source IDs keep those symbols distinct. Overloads remain separate when their normalized signatures differ. Duplicate symbol IDs within one catalog are invalid.

An optional source `collection` object records the collection profile, outcome, timestamps, stable error code, and truncation flag. Outcomes are `succeeded`, `succeeded-with-truncation`, `unavailable`, `permission-denied`, `failed`, `skipped-by-profile`, and `not-configured`. The `truncated` flag is true exactly for `succeeded-with-truncation`. The optional package-level `completeness.status` is `complete`, `partial`, or `unknown`; `complete` requires every included source to have a successful, non-truncated collection outcome. `partial` requires at least one explicitly non-successful source outcome. Legacy catalogs without these additive fields remain readable and have unknown completeness.

Evidence records may include `revision` and `location`. When evidence names a revision, it must match the revision of its referenced source. A symbol's `sourceId` remains its primary attribution; additional evidence may refer to compatible sources without overwriting the primary source.
