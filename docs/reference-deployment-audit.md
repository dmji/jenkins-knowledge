# Reference deployment lifecycle audit

**Date:** 2026-11-27
**Audited revision:** `c34ff1077ae0a3a674b70210712e36c754e58080` (master, merged PR #117)
**Scope:** implementation and repository documentation at the audited revision. This is a code/documentation audit, not evidence of a live Jenkins compatibility run.

## Executive summary

The repository already implements most of the package lifecycle foundation: bounded collection from one configured Jenkins controller, optional plugin inventory, Pipeline GDSL extraction, Shared Library scanning, collection profiles, package provenance/completeness, atomic package publication, and offline Runtime/MCP/LSP consumers. The Runtime/LSP crates do not depend on an HTTP client, and the VS Code extension delegates to the local LSP.

The largest unproven parts are **live compatibility with Jenkins 2.356 and the selected Java/plugin matrix**, and **a reproducible end-to-end acceptance path that includes a real Jenkins and the installed VS Code extension**. A complete operational/user guide and demo environment are also absent.

Do not create replacement work for the existing collection/provenance/completeness/runtime functionality. The existing issues #119–#125 should be narrowed using the status and evidence below before implementation proceeds.

## Lifecycle matrix

| Lifecycle area | Status | Evidence and boundaries |
|---|---|---|
| Compatibility/prerequisites | **Partial** | Builder validates profile syntax, controller URLs, API roots and limits, and exposes a network-free collection plan. It records observed Core/plugin metadata but does not compare a target Jenkins against a tested compatibility matrix. No Jenkins 2.356 live test is present. |
| Credentials and permissions | **Partial** | Builder accepts paired `JENKINS_USERNAME` / `JENKINS_API_TOKEN` environment variables; credentials are not serialized. Quick/standard are documented for a read-only account; optional plugin inventory can be denied. Deep Script Console is explicit and requires administrator-level permission. These permission claims are documented, not verified against a live authorization strategy. |
| Controller connection/discovery | **Implemented (one controller per package)** | `jenkins-gdsl-collector/src/discovery.rs` performs bounded sequential requests to `/api/json`, `/pluginManager/api/json`, and a one-byte GDSL probe. Core version is read from `X-Jenkins` or `jenkinsVersion`; plugin inventory is optional and preserves unavailable/unknown states. Builder rejects multiple configured controllers because the catalog has one top-level controller snapshot. |
| Shared Libraries | **Implemented** | Builder scans configured Git remotes in standard/deep, records the resolved commit SHA, and isolates per-library failures. Quick intentionally skips library scanning. See `crates/shared-library-scanner` and `crates/knowledge-builder/README.md`. |
| Profiles and collection | **Implemented** | Quick, standard and deep have distinct request budgets and surface boundaries. The network-free `knowledge-builder plan` reports requests, limits, remotes and credentials source without reading credential values or making calls. Deep is the only profile that invokes the fixed repository-owned Script Console probe. |
| Package validation/publication | **Implemented** | Builder merges and validates sources, writes `catalog.json` and `manifest.json`, records checksums and per-surface outcomes, and publishes through a same-filesystem staging rename. Runtime verifies declared checksums, schemas, references and unsafe paths before loading. |
| Provenance/completeness | **Implemented, with Java metadata caveat** | Manifest collection records profile, timestamps, package status and per-surface outcomes. Catalog records source revisions/fingerprints, coverage, snapshot identity, controller Core/plugin inventory, and optional runtime metadata. Legacy additive fields remain optional. Java version is currently collected only by the opt-in deep Script Console probe. |
| Runtime / CLI | **Implemented** | Runtime opens an explicit package, validates it, and exposes symbols, controller/plugin inventory, source details, collection outcomes and coverage. Its dependency manifest contains no HTTP client; runtime code does not perform Jenkins/Git collection. |
| MCP | **Implemented** | MCP is a local stdio consumer of Runtime and exposes read-only tools/resources. Cross-component contract tests exercise it using the Builder-produced fixture package. |
| LSP | **Implemented** | LSP is local and package-backed. It implements completion, hover, definition and diagnostics, and exposes collection/controller/plugin/source requests. Unit and cross-component tests cover these behaviors. |
| VS Code | **Partial** | The extension launches the configured local LSP, passes package path/profile, and provides package reload. CI compiles the TypeScript extension; there is no automated VS Code host/extension installation test or full user acceptance test. |
| Refresh and operations | **Partial** | Refresh is explicit; controller snapshots are cached and invalidated on changed fingerprints. Builder refuses to overwrite an existing package. Documentation does not yet provide a complete operator lifecycle for package promotion, versioning, backup/restore and troubleshooting. |
| Documentation and onboarding | **Partial** | README, architecture, Builder, Runtime, collector, controller validation and cross-component test docs exist. There is no reference-deployment guide/navigation or separate demo-environment guide. The default collection-profile documentation is inconsistent (see findings). |

## Detailed evidence

### Builder and controller collection

- `crates/jenkins-gdsl-collector/src/discovery.rs:94-168`: quick discovery makes sequential bounded Core and plugin inventory requests and probes GDSL availability. The Core version comes from the Jenkins response header or JSON field; no version is inferred from another version.
- `crates/jenkins-gdsl-collector/src/discovery.rs:170-257`: plugin inventory is optional and distinguishes available, unavailable and unknown results.
- `crates/knowledge-builder/src/lib.rs:102-104`: the actual default `collectionProfile` is **quick**.
- `crates/knowledge-builder/src/lib.rs:614-645`: standard/deep download GDSL; deep additionally schedules the fixed Script Console probe.
- `crates/knowledge-builder/src/lib.rs:649-665`: Shared Libraries are scanned outside quick; Script Console is only selected in deep.
- `crates/knowledge-builder/src/lib.rs:771-781`: live Builder collection obtains controller metadata before optional sources.
- `crates/knowledge-builder/src/lib.rs:983-1036`: deep-only Script Console collection and profile-specific request counts.
- `crates/knowledge-builder/README.md`: documents profile semantics, controller cache, atomic package layout, explicit refresh, Shared Library revisions and bounded API roots.

### Environment metadata and package contract

- `schemas/catalog.schema.json:508-562`: optional controller snapshot includes identity, `coreVersion`, plugin inventory, capabilities and collection metadata.
- `schemas/catalog.schema.json:926-1008`: optional controller runtime metadata supports `javaVersion`, Java vendor, Groovy version, extension registries and API type inventory.
- `crates/jenkins-gdsl-collector/src/probes/script_console_v1.groovy:35-64`: Java version/vendor and Groovy version are read from JVM/Groovy system properties inside the Script Console probe. This is not part of quick/standard.
- `schemas/manifest.schema.json`: additive `collection` records package status, profile, start/completion timestamps and per-source surface outcomes.
- `docs/catalog-contract.md`: distinguishes manifest execution outcomes from catalog coverage, provenance and symbol confidence; documents unknown/partial semantics and stable snapshot/fingerprint rules.
- `crates/knowledge-builder/tests` and `crates/jenkins-gdsl-collector/tests/fixtures/controller`: sanitized Core, plugin-manager and Script Console response fixtures cover positive and partial/denied outcomes.

### Runtime, MCP, LSP and VS Code

- `crates/jenkins-runtime/Cargo.toml`: Runtime dependencies are local validator, JSON Schema, serialization and hashing libraries; no HTTP/Git client.
- `crates/jenkins-lsp/Cargo.toml`: LSP depends on Runtime and the stdio LSP protocol library; no HTTP/Git client.
- `crates/jenkins-runtime/src/main.rs`: CLI exposes controller, plugins, sources and coverage in addition to symbol queries.
- `crates/jenkins-lsp/src/main.rs`: completion, hover, definition, diagnostics and collection/controller/plugin requests are implemented and tested.
- `crates/jenkins-contract-tests/tests/cross_component.rs`: builds the same fixture package through Builder and exercises Runtime, LSP and MCP, including profile isolation, partial collection and unavailable-plugin diagnostics.
- `editors/vscode/src/extension.ts` and `editors/vscode/package.json`: extension is a thin LSP client with package path/profile settings and a reload command. Current CI compiles it; it does not launch a VS Code test host.

## Findings requiring correction or follow-up

1. **Documentation default mismatch (confirmed).** Code defaults `collectionProfile` to `quick` (`crates/knowledge-builder/src/lib.rs:102-104`) and the root README says quick (`README.md:110`), but the Builder profile table says standard (`crates/knowledge-builder/README.md:68-72`). Correct the Builder README to quick; do not change runtime behavior.
2. **Java version availability is intentionally profile-dependent but not fully represented.** Java version/vendor are available from the fixed Script Console probe only (`script_console_v1.groovy:55-64`), hence only deep can populate them. Quick/standard do not have an equivalent proven read-only endpoint in this implementation. Keep the field unknown/not collected rather than inferring it; make this limitation/status explicit in package/runtime reporting and tests. Do not require Script Console for basic profiles.
3. **Jenkins 2.356 compatibility remains unverified.** Existing tests use sanitized fixtures and local HTTP mocks. PR #117 explicitly says no live Jenkins controller was contacted. A reproducible environment and permissioned test against 2.356 are still required before claiming compatibility.
4. **End-to-end coverage stops short of the user's installed VS Code.** Existing CI validates Builder→Runtime→LSP→MCP with offline fixtures and compiles the extension. It does not install/launch VS Code, validate extension activation, or run the full workflow against a live Builder-produced package.
5. **Operational docs and demo environment are absent.** Existing documentation is developer/operator-oriented and fragmented. There is no checked-in Docker Compose/demo Jenkins environment and no complete reference deployment guide for an existing customer Jenkins.
6. **CI cannot be reproduced on the currently connected Desktop Commander host.** The checkout has Node 22.23.4/npm 10.9.9 and no Rust toolchain; `scripts/validate.sh` requires Rust 1.100.0 and Node 26.13.1/npm 11.20.0. The script exits before tests. Use GitHub Actions as the authoritative validation path; do not interpret this local prerequisite failure as a code/test failure.

## Existing issues: recommended scope after this audit

| Issue | Audit disposition |
|---|---|
| #119 — environment compatibility metadata | **Narrow to the Java metadata availability contract and tests.** Core version and optional plugin inventory are already collected in quick discovery; profile/time/outcome and source provenance are already in the package. Java version is only available through deep Script Console today. Do not duplicate Core/plugin collectors or introduce an Administer requirement for quick/standard. |
| #120 — Jenkins 2.356 validation | **Valid, not implemented.** Add a reproducible Jenkins 2.356 test target, pin the actual Java/plugin set, run all three profiles with the documented permission boundaries, and record sanitized results. |
| #121 — package provenance/completeness contract | **Largely implemented.** Manifest collection outcomes, catalog controller/Core/plugin data, revisions, fingerprints, coverage, snapshot ID and completeness already exist. Review only the Java availability representation and any demonstrated schema gap; avoid a second provenance model. |
| #122 — Runtime compatibility reporting | **Largely implemented.** Runtime exposes controller/plugin inventory, source details, coverage and collection status. It must report provenance/completeness and avoid asserting target compatibility without target evidence; add only a demonstrated missing presentation/API field. |
| #123 — end-to-end acceptance | **Partially implemented.** Keep the existing Builder→Runtime→LSP→MCP contract suite. Extend it with the live Jenkins test artifact and VS Code extension-host acceptance; do not replace the existing suite. |
| #124 — reproducible demo environment | **Not present.** Separate optional local demo from integration with an existing user Jenkins. Pin image/Java/plugin/library revisions and provide safe reset/cleanup. |
| #125 — integration and operations guide | **Partially present.** Reuse existing README/architecture/Builder/Runtime/controller-validation pages and add navigable lifecycle/operations guidance. Fix the confirmed profile-default contradiction. |

## Recommended order

1. Fix the confirmed Builder README default mismatch and define the Java metadata unknown/not-collected behavior without raising base-profile permissions (#119).
2. Validate the real Jenkins 2.356 + Java + plugin matrix (#120); use its sanitized package as the integration-test input.
3. Make only evidence-backed contract/runtime changes arising from #119/#120 (#121/#122); close or reduce already-satisfied scope rather than duplicate it.
4. Extend the existing cross-component test to VS Code host acceptance and the real collected package (#123).
5. Add the optional pinned demo environment (#124).
6. Publish the reference integration and operations guide against the tested implementation (#125).

## Explicit non-findings

The audit found no basis for adding mandatory package signatures for an exclusively trusted internal workflow, no need for Runtime to contact Jenkins/Git, and no need to install/manage Jenkins or install plugins on the user's controller. These would contradict the implemented trust boundary rather than close a demonstrated gap.
