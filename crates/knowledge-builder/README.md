# Knowledge Builder

Build a versioned Knowledge Package from an explicit YAML profile. Builder is the administrator-operated collection stage; its profile defines the only Jenkins controllers and Git repositories that are contacted. Running Runtime, LSP, or MCP never starts or refreshes a Builder job.

## Offline fixture build

The profile may provide `fixtures`, a list of catalog JSON paths relative to the profile file. This mode is intended for tests and reproducible builds; it does not contact Jenkins or Git.

```sh
cargo test --manifest-path crates/knowledge-builder/Cargo.toml --all-targets
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- build crates/knowledge-builder/tests/fixtures/profile.yml ./target/fixture-package
```

## Live collection

Without `fixtures`, the Builder first requires bounded Jenkins Core/controller metadata, then records each optional collection surface independently. Jenkins credentials are read from `JENKINS_USERNAME` and `JENKINS_API_TOKEN`; provide both or neither. Git credentials use the normal Git credential helper or SSH agent. Never embed credentials in profile URLs. Controller URLs must use HTTP(S), and Git remotes must use HTTPS, SSH, or Git SCP syntax. URL user information, query parameters, and fragments are rejected; Git references that could be interpreted as command-line options or malformed refs are rejected before any network operation.

### Collection profiles

`collectionProfile` is separate from the package's `profile` identity. It defaults to `standard` and accepts:

| Collection profile | Required sources | Optional or skipped surfaces |
|---|---|---|
| `quick` | Jenkins Core/controller metadata | Plugin inventory is optional; GDSL is availability-probed but not downloaded; configured Shared Libraries and Script Console are skipped. |
| `standard` | Jenkins Core/controller metadata | Plugin inventory, full GDSL extraction, and configured Shared Libraries are optional. Script Console is skipped. |
| `deep` | Jenkins Core/controller metadata | Plugin inventory, full GDSL, configured Shared Libraries, and the fixed read-only Script Console probe are optional. |

The Declarative Pipeline model is generated locally for each configured controller. Controller metadata is mandatory because it establishes the identity and version anchor for controller-backed sources. A failure there aborts the build. Optional failures do not discard successful snapshots: the Builder publishes a partial package with stable status and error codes. A failed Shared Library scan is isolated to that library; other controllers/libraries remain available. The Script Console probe runs only in `deep`, uses repository-owned Groovy, and does not accept caller-supplied scripts. It discovers a bounded API type surface rooted in observed Pipeline/Declarative classes and optional per-controller `apiRoots`; it never enumerates a ClassLoader or scans packages/JARs.

Retries are explicit rather than automatic to avoid multiplying controller load. Network timeouts and transient HTTP failures are safe to retry by rerunning the build. Permission failures require corrected Jenkins/Git permissions or credentials first. Malformed responses should be investigated against the Jenkins/plugin version before retrying. The package records no raw remote response bodies or error text.

Shared Library `commit` values, when supplied, must be full 40-character commit SHAs. The scanner records the resolved commit in the catalog source revision. When omitted, the configured ref is fetched and the resolved commit is recorded by the scanner.

## Bounded Jenkins API type inventory

Deep collection roots API inspection only in classes already observed from Pipeline Steps, Global Variables, Declarative descriptors, plus explicitly configured `jenkins.controllers[].apiRoots`. The explicit list accepts at most 20 fully-qualified Java class names (for example `hudson.model.Run`); arbitrary Groovy, package wildcards, JAR paths, and caller-provided scripts are rejected.

Traversal is breadth-first. Per-controller `apiLimits` can lower the defaults (40 types, depth 2, 12 methods per type, 300 methods total); hard maxima cannot be raised. Parameter/property caps (20 each) and the 256 KiB Script Console output limit remain fixed. The collector reads class/member metadata only: it does not invoke methods or constructors, enumerate ClassLoaders, or scan packages/JARs. Superclasses, interfaces, overloads, generic signatures, parameter names where present, plugin attribution, class-origin basenames, and descriptor help-file paths where Jenkins exposes them are retained. Help paths are metadata references only; the collector does not fetch help-file contents. Bean properties are inferred only from public/protected getter/setter signatures and are marked `confidence: inferred`.

Synthetic and bridge methods are explicitly marked; unresolved classes and inaccessible member metadata remain partial/unknown. Reflection presence is not evidence of Jenkins Script Security Sandbox approval or Pipeline CPS compatibility. Groovy language/runtime APIs remain distinct from controller-specific Jenkins/plugin classes.

## Profile contract

Profiles use strict YAML deserialization: unknown keys at the root or in any nested configuration object are errors. This prevents spelling mistakes from being silently ignored.

| Field | Default / requiredness | Effect |
|---|---|---|
| `profile` | Required, non-empty | Package profile identity written to `manifest.json`; used as the catalog ID when `package.id` is omitted. |
| `collectionProfile` | Defaults to `standard` | Collection policy: `quick`, `standard`, or `deep`; independent of the package profile identity. |
| `jenkins.controllers` | Empty when omitted; at most one supported | The controller ID scopes source identity and its URL is the HTTP(S) endpoint. Optional `apiRoots` lists at most 20 fully-qualified Java class names for explicit type roots; roots are validated before collection and are inspected only in `deep`. Multiple controllers are rejected before collection because the current catalog schema has one top-level controller snapshot. |
| `libraries` | Empty when omitted | Each Shared Library is scanned once. `name`, `url`, and `revision` are required; optional `commit` pins the fetch to a full 40-character SHA while retaining the configured revision as metadata. Duplicate names are rejected. |
| `fixtures` | Empty when omitted | Offline catalog JSON paths, relative to the profile file. Fixture mode is mutually exclusive with live controllers/libraries. Duplicate and absolute paths are rejected. |
| `package.id` | Omitted | Catalog ID defaults to `profile`; a supplied ID must be non-empty and have no surrounding whitespace. |
| `package.include` | Omitted means all sections | Selects package sections before merge/serialization. An explicit empty list, duplicates, unknown values, and selectors matching no sources are errors. |

Supported `package.include` values:

- `jenkins-core`: Jenkins Core version source observed from each configured controller (or explicit `jenkins-core` sources in fixtures).
- `controller`: controller source, controller snapshot, and Declarative model sources.
- `plugins`: plugin sources, their symbols, and the controller's plugin inventory.
- `shared-libraries`: Shared Library sources and symbols.

Filtering is applied by source ID: excluded sources, their symbols, and their coverage entries are removed together. When `controller` is selected without `plugins`, the controller snapshot is retained but its nested plugin inventory is empty. Omit `package.include` to include every available section; `include: []` is not an alias for the default.

**Migration note:** earlier Builder versions accepted `package.include` but ignored it. It now actively filters the package. To preserve the previous all-sections output, remove `package.include` rather than setting it to an empty list. Review existing explicit selectors before upgrading; every requested section must match at least one source.

A profile must select at least one source mode. Fixture catalogs cannot be mixed with live collection. Controller IDs and library names must be unique. URL, ref, commit, fixture-path, package-ID, and selector validation completes before any external collection starts. A requested section must match at least one source after collection; an unavailable plugin inventory therefore fails an explicit `plugins` selection instead of silently producing a partial package.

## Package layout and guarantees

The output directory contains `catalog.json` and `manifest.json`. The manifest records package/catalog schema versions, profile identity, build timestamp, source revisions, source provenance, tool version, and a SHA-256 checksum of the catalog. Its additive `collection` object records package completeness (`complete` or `partial`), collection profile, start/completion timestamps, and one outcome per configured surface. Outcomes distinguish `succeeded`, `succeeded-with-truncation`, `partial`, `unavailable`, `permission-denied`, `failed`, `skipped-by-profile`, and `not-configured`; stable `errorCode` values are machine-readable and never contain raw remote diagnostics. Catalog sources and symbols are sorted by ID before serialization.

Set `SOURCE_DATE_EPOCH=0` for a stable timestamp and byte-identical fixture package output. Builds fail with the source label on collection or validation errors. The Builder writes into a unique temporary directory on the output filesystem and publishes it with a same-filesystem rename only after validation and serialization. Temporary checkouts and staging directories are managed by RAII and are removed on every early return, including failed Git operations and serialization/write errors. It refuses to overwrite an existing output directory. Controller snapshot identity, Core version, plugin inventory, capabilities, and collection metadata are retained; builds reject incompatible controller snapshots instead of silently combining them. Source URLs are sanitized before entering catalogs and manifests so user information and URL request modifiers are not persisted.

```sh
cargo test --manifest-path crates/knowledge-builder/Cargo.toml --all-targets
cargo clippy --manifest-path crates/knowledge-builder/Cargo.toml --all-targets -- -D warnings
```
