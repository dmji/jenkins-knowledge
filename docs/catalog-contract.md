# Catalog contract

The catalog is the canonical API representation shared by Builder, Runtime, IDE tooling, and MCP. The JSON Schema in `schemas/catalog.schema.json` is normative; generated stubs and downstream indexes must not add API facts absent from the catalog.

## Top-level fields

- `schemaVersion`: catalog schema version. Version 1 is `1.0.0`.
- `catalogId`: stable identifier for this catalog snapshot.
- `generatedAt`: RFC 3339 timestamp.
- `sources`: source snapshots from which symbols were extracted.
- `symbols`: API symbols associated with exactly one source.
- `coverage` (optional): per-source collection status for an inspected API surface.
- `snapshotId` (optional): stable SHA-256 identity of the logical catalog content, excluding only documented volatile timestamps.
- `completeness` (optional): `complete`, `partial`, or `unknown`, with stable reason codes when partial.

## Sources

Each source has a unique `id`, a `kind`, a human-readable `name`, and a `revision`. Shared Library revisions must identify the resolved Git commit SHA; the configured branch or tag belongs in `version` or source metadata. Controller revisions identify the Jenkins version/snapshot used for extraction. Optional URLs and metadata provide traceability but must not contain credentials.

## Controller runtime snapshot

The optional top-level `controller` object describes one concrete Jenkins controller. It is additive to schema 1.0.0: existing GDSL-only and Shared Library catalogs remain valid when it is absent.

- `identity` contains a stable controller identifier and optional display name/base URL. URLs must be credential-free.
- `coreVersion` records the observed Jenkins Core version.
- `plugins` is a bounded inventory of installed plugins. Each entry references a `plugin` source and records the short name, observed version, enabled/active state (nullable when unknown), and declared dependencies. A missing plugin is not implied by an incomplete inventory.
- `capabilities` records named probes as `available`, `unavailable`, or `unknown`; it does not infer capability from a Core version.
- `collection` records the profile, retrieval timestamp, optional start/completion timestamps, and request count. It may also record `javaVersionStatus` and `javaVendorStatus`: `available`, `not-collected-by-profile`, `unavailable`, or `probe-failed`. These are per-property outcomes, not package-level collection statuses.

The optional `runtime` object records Deep Script Console observations. `javaVersion` and `javaVendor` are included only when read from the actual JVM system properties. Their per-property statuses live in `collection`: `available` means a non-empty value was observed; `not-collected-by-profile` means Quick/Standard intentionally skipped Script Console; `unavailable` means the probe ran but did not provide the property (or Script Console itself was unavailable); `probe-failed` means reading that property threw an error. A Java property failure is field-local and does not make an otherwise successful Script Console snapshot partial. Never infer Java from Jenkins Core.

A symbol may include `availability` and `executionConstraints`. Availability is explicitly `available`, `unavailable`, or `unknown`, with optional plugin source references. CPS and Script Security/Sandbox values distinguish positive/negative observations from `unknown` and `not-inspected`. These fields must only be populated from evidence; the presence of a method or step does not prove CPS compatibility or Sandbox approval.

The optional `evidence` array attaches source identity, evidence class (`controller-observed`, `source-scanned`, `static-model`, or `inferred`), confidence, retrieval time, and an optional evidence location to an individual symbol. Evidence source IDs and plugin availability references must resolve to catalog sources. Existing `sourceId`, `provenance`, `confidence`, and `fieldEvidence` remain supported for older producers.

## Package collection outcomes

The manifest's optional `collection` object is the package-level execution record; catalog `coverage` remains the per-source/API-surface observation record. These are related but not interchangeable. The manifest records the selected collection profile, start/completion timestamps, package status (`complete`, `partial`, or `unknown`), and one outcome per configured collection surface.

Collection outcome statuses are:

- `succeeded`: the configured surface was collected successfully.
- `succeeded-with-truncation`: usable data was retained but a documented bound truncated the result.
- `partial`: the surface returned usable data alongside one or more bounded probe failures.
- `unavailable`: the endpoint or source could not be reached or is not exposed.
- `permission-denied`: Jenkins or Git explicitly rejected access.
- `failed`: the source was configured but its response, parsing, or collection failed.
- `skipped-by-profile`: the selected quick/standard/deep profile or package section intentionally omitted the surface.
- `not-configured`: the surface is not configured for this package.
- `unknown`: no explicit outcome is available for the source; this must not be interpreted as success.

The package is `partial` if any outcome is truncated, partial, unavailable, permission-denied, or failed. A skipped or not-configured surface does not make the selected package incomplete. Controller metadata is mandatory in all live Jenkins profiles; failure aborts the build because no safe controller identity/version anchor exists. Plugin inventory is optional in all profiles. Quick probes GDSL availability only; standard and deep attempt full GDSL collection. Standard and deep scan configured Shared Libraries independently. Deep additionally executes the fixed repository-owned read-only Script Console probe; Script Console permission failure does not discard REST/GDSL results.

Each outcome carries a stable ID, source kind, display name, surface, status, and optional catalog source ID/revision/error code. Error codes are stable machine values; raw exception strings, HTTP bodies, request headers, credentials, and response fragments are never stored. Runtime, MCP, and LSP preserve these outcomes as distinct from symbol confidence. Consumers must not infer complete coverage from successful package loading or from a symbol marked verified.

Retry guidance is profile-aware and explicit: retry timeouts and transient server errors by rerunning collection; correct permissions before retrying authorization failures; investigate malformed or incompatible responses before repeating the same request. The Builder does not automatically retry because that would increase Jenkins load and make request counts nondeterministic.

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
- **Coverage** describes what collection actually inspected. The optional top-level `coverage` array records a `sourceId`, a `surface`, and one status: `inspected`, `succeeded`, `succeeded-with-truncation`, `partial`, `unavailable`, `permission-denied`, `failed`, `skipped-by-profile`, `not-configured`, `unsupported`, or `not-collected`. Coverage entries must reference an existing source. Absence of a coverage entry is not evidence that a surface was inspected.

The optional symbol `fieldEvidence` object records evidence quality independently for individual fields, using the same `verified`, `inferred`, and `unknown` values. A missing field-evidence entry means unknown; symbol-level `confidence: verified` must not be interpreted as proof that every optional attribute is verified. Producers should only mark a field verified when its value was directly observed or declared by the referenced source.

## Package format, build identity, and checksums

`manifest.packageVersion` identifies the Knowledge Package/manifest format; the currently supported value is `1.0.0`. `manifest.catalogSchemaVersion` must equal `catalog.schemaVersion` and currently supports `1.0.0`. Runtime rejects unsupported versions instead of attempting best-effort interpretation. Additive optional fields may be absent from older 1.0.0 packages; consumers treat absent collection/completeness/runtime observations as unknown. Required source identity, `sourceRevisions`, and `provenance` reconciliation remain part of the current package contract. Breaking field, enum, or semantic changes require an explicit package/catalog version decision and migration notes.

`manifest.createdAt` records package creation time; `manifest.collection.startedAt/completedAt` bound the collection run. `catalogId` identifies the logical catalog namespace and `snapshotId` identifies its logical content. Together these provide build time and content identity without adding a second, redundant build identifier. `manifest.toolVersions` records the producing tool versions.

`manifest.checksums` contains SHA-256 digests of the exact serialized bytes of package payload files (currently `catalog.json`). The manifest is not self-checksummed. `snapshotId` is different: it hashes canonical logical catalog content and excludes only documented volatile timestamps. Therefore a new retrieval time can change the file checksum while preserving the same logical snapshot ID. JSON object keys and identity-bearing arrays are canonicalized before snapshot hashing; Builder serialization and package checksums use deterministic JSON serialization. `sourceRevisions` is retained as a v1 compatibility index; `provenance` carries the corresponding source identity record. Both must match the authoritative catalog source and are validated rather than treated as independent facts.

## Validation and compatibility

Validation applies JSON Schema constraints and semantic rules: unique source IDs and symbol IDs, valid symbol and coverage source references, one coverage record per source/surface pair, unique parameter names within a symbol, and rejection of credential-bearing source URLs or secret-named fields. Duplicate source identities are rejected during package merge rather than overwritten. Same-name symbols from distinct sources remain distinct when their IDs and source references are distinct. Conflicts are reported with both contributing input labels and revisions.

Unknown properties are rejected. The optional `coverage` and `fieldEvidence` fields are additive and do not invalidate existing 1.0.0 catalogs. Changes to required fields, enum values, or field meaning require an explicit schema version decision and migration notes. The current schema rejects unsupported schema versions rather than guessing.

## Stable snapshot identity and source collection outcomes

`catalogId` identifies the logical catalog namespace. `snapshotId` is a SHA-256 digest (`sha256:` prefix) of the canonical logical catalog. It excludes only the explicitly volatile timestamp fields `generatedAt`, `createdAt`, `startedAt`, `completedAt`, `retrievedAt`, and `collectedAt`; these fields remain available for diagnostics. Object keys are canonicalized by JSON serialization, and identity-bearing arrays (sources, symbols, coverage, plugins, capabilities, dependencies, evidence, and extension registry entries) are normalized by stable identifiers before hashing. Changes to any other field change the snapshot ID.

Each source `id` identifies a logical configured source and remains stable as its revision advances. A Shared Library identity is derived from its configured repository/name/reference, while `revision` is the resolved Git commit. Controller discovery uses a digest of the normalized observed controller/plugin snapshot. GDSL collection uses a digest of the retrieved DSL document. Static Declarative models use their model version. Collection timestamps are never revisions. `fingerprint` is a SHA-256 digest of the source record, its primary and secondary evidence symbols, coverage entries, and associated controller/plugin observation.

Symbol IDs are deterministic within a source: source identity + symbol kind + namespace/owner + name + normalized signature. Line numbers and retrieval timestamps belong in provenance, not identity. Overloads remain distinct when signatures differ. Identical duplicate IDs within one source are invalid; equal names from different sources remain distinct because source identity is part of the ID. Secondary evidence contributes to the fingerprint of both the primary and referenced evidence sources.

Each source may carry a `collection` summary (`profile`, `status`, `startedAt`, `completedAt`, `errorCode`, `truncated`). It aggregates the per-surface outcomes recorded in `manifest.collection.sources`. Statuses are `succeeded`, `succeeded-with-truncation`, `partial`, `unavailable`, `permission-denied`, `failed`, `skipped-by-profile`, `not-configured`, and `unknown`. `truncated` is true when any source surface was truncated. Profile-skipped and not-configured surfaces are expected exclusions; they do not by themselves make the package partial. A source with mixed successful and failed surfaces is `partial`.

Package `completeness.status` is `complete`, `partial`, or `unknown`. `complete` means every catalog source has an explicit outcome and no source/global collection outcome failed, was unavailable, permission-denied, or truncated. `partial` requires explicit outcomes for every catalog source and at least one failed, unavailable, permission-denied, or truncated outcome; `reasonCodes` records the stable reasons. `unknown` is used when a catalog source has no outcome. The manifest collection status and catalog completeness status must agree. Legacy packages without these additive fields remain readable.

Evidence records may include `revision` and `location`. When evidence names a revision, it must match the revision of its referenced source. Provenance records in the manifest carry the same source fingerprint as the catalog source. These hashes identify content; they are not signatures or proof of authenticity.
