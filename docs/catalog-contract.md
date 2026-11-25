# Catalog contract

The catalog is the canonical API representation shared by Builder, Runtime, IDE tooling, and MCP. The JSON Schema in `schemas/catalog.schema.json` is normative; generated stubs and downstream indexes must not add API facts absent from the catalog.

## Top-level fields

- `schemaVersion`: catalog schema version. Version 1 is `1.0.0`.
- `catalogId`: stable identifier for this catalog snapshot.
- `generatedAt`: RFC 3339 timestamp.
- `sources`: source snapshots from which symbols were extracted.
- `symbols`: API symbols associated with exactly one source.

## Sources

Each source has a unique `id`, a `kind`, a human-readable `name`, and a `revision`. Shared Library revisions must identify the resolved Git commit SHA; the configured branch or tag belongs in `version` or source metadata. Controller revisions identify the Jenkins version/snapshot used for extraction. Optional URLs and metadata provide traceability but must not contain credentials.

## Symbols

Each symbol has a unique `id`, a non-empty `name`, a `kind`, a `sourceId` referencing a catalog source, and a `confidence`. Optional fields describe namespace, signature, description, parameters, return type, overloads, examples, constraints, contexts, documentation URL, and provenance.

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

## Validation and compatibility

Validate JSON Schema constraints and semantic references (unique source IDs, unique symbol IDs, and valid `sourceId` references). Unknown properties are rejected. Schema changes that add optional fields may be compatible; changes to required fields, enum values, or field meaning require an explicit schema version decision and migration notes. The current schema rejects unsupported schema versions rather than guessing.
