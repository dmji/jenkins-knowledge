# Jenkins GDSL collector

Collects a bounded Jenkins controller snapshot through read-only REST endpoints,
or extracts the controller-specific Pipeline DSL from Jenkins' authenticated
/pipeline-syntax/gdsl endpoint.

## Quick discovery (default)

The CLI defaults to the quick profile. It identifies the controller, reads the
installed plugin inventory when permitted, and probes whether Pipeline GDSL is
available without downloading the GDSL document:

    JENKINS_USERNAME=ci-reader JENKINS_API_TOKEN="$JENKINS_API_TOKEN" \
      cargo run --manifest-path crates/jenkins-gdsl-collector/Cargo.toml -- \
      https://jenkins.example.test/jenkins ./controller.json

The quick profile performs exactly three sequential requests:

1. GET /api/json?tree=_class,nodeName,jenkinsVersion for bounded controller
   metadata and the X-Jenkins Core version header.
2. GET /pluginManager/api/json?tree=plugins[shortName,version,active,enabled,dependencies[shortName,version,optional]]
   for the installed plugin inventory.
3. GET /pipeline-syntax/gdsl with Range: bytes=0-0, reading at most one
   response byte to determine endpoint availability.

The profile uses a 5-second connection timeout and a 15-second total timeout.
Core metadata is limited to 1 MiB, plugin inventory to 4 MiB, and the GDSL
availability probe reads at most one byte. Plugin inventory is limited to 5,000
plugins and 256 dependencies per plugin. Requests are sequential, no polling or
background refresh is performed, and no automatic retries are issued. Redirects
are not followed, preventing credentials from being forwarded to another host.

The output is a schema-validated catalog with a controller snapshot, plugin
sources, collection profile/timestamps/request count, and explicit capability
and coverage results. A 401/403 or 404/410 on an optional endpoint is recorded
as unavailable. Timeouts, server errors, and malformed optional plugin data are
recorded as unknown/not-collected; they are not interpreted as proof that a
feature or plugin is absent. The mandatory /api/json endpoint fails the run
on authorization, HTTP, timeout, size, or JSON errors.

## Full GDSL extraction

The existing GDSL collector remains available explicitly:

    JENKINS_USERNAME=ci-reader JENKINS_API_TOKEN="$JENKINS_API_TOKEN" \
      cargo run --manifest-path crates/jenkins-gdsl-collector/Cargo.toml -- \
      https://jenkins.example.test/jenkins ./catalog.json --profile gdsl

The Rust API function collect remains unchanged for existing Builder callers.
The GDSL parser extracts explicit method(name: ...) declarations; it does not
execute Groovy, evaluate dynamic expressions, or infer missing APIs. Parameter
descriptions, return types, and documentation URLs are retained only when
explicitly represented in the declaration. Jenkins version is recorded from
the X-Jenkins response header when present. Plugin-specific steps are included
when the installed controller's GDSL exposes them.

Before locating declarations, a bounded lexical pass masks line comments,
nested block comments, single/double-quoted strings, triple-quoted strings,
slashy strings, and dollar-slashy strings while preserving source offsets and
line numbers. Escaped delimiters are honored. Method parentheses and parameter
lists are then matched and split only in code regions, so nested maps/lists and
quoted commas, colons, brackets, and parentheses do not truncate a declaration.
Unclosed comments, strings, or parameter lists return a line-specific
MalformedGdsl error rather than producing partial or fabricated symbols.

This is deliberately a small GDSL-oriented scanner, not a complete Groovy
parser. String contents are opaque (including GString interpolation), dynamic
expressions are not evaluated, and only explicit named arguments and
string-valued descriptions/types are retained. Parsing is O(n) in the response
size with O(n) auxiliary memory; the 4 MiB response cap, 10,000-symbol cap,
and 256-parameter-per-symbol cap bound that work.

Each symbol is marked verified because its name is backed by an explicit
controller declaration. Provenance identifies the endpoint and source line.
The source revision records retrieval time because GDSL does not expose a stable
content revision. A controller snapshot should therefore be regenerated when
Jenkins or its plugins change.

## Authentication and permissions

The controller URL must be HTTP(S) and must not contain embedded user information.
Credentials are supplied separately through JENKINS_USERNAME and
JENKINS_API_TOKEN, either both or neither. They are sent using HTTP Basic
authentication and are never written to output or diagnostics. Query strings
and fragments are removed from the recorded controller URL. Use HTTPS in
production and inject credentials through a secret manager or environment.

Grant the service account Jenkins Overall/Read for controller metadata. The
plugin inventory and Pipeline GDSL endpoints may be further restricted by the
controller's authorization strategy or installed security plugins. The quick
profile records 401/403 responses for those optional endpoints as unavailable;
it does not require Script Console access or request Overall/Administer.
Do not broaden permissions solely to make an optional capability appear
available.

## Compatibility and evidence

The full GDSL collector targets Jenkins installations exposing the Pipeline
Syntax GDSL endpoint (Pipeline: Groovy / workflow-cps plugin family). GDSL is
Groovy DSL source, not XML. It records Jenkins Core version from the response
header when present and preserves only explicitly declared metadata.

The quick profile records observed controller metadata and plugin inventory.
Unknown plugin state remains null; missing version/dependency facts are not
inferred. The quick profile only probes GDSL availability and does not collect
its symbols. Runtime facts and static/inferred model data remain separate.

HTTP 401/403, other non-success responses, empty responses, invalid UTF-8,
invalid URLs, oversized responses, timeouts, and malformed GDSL declarations
produce structured errors in full GDSL mode. Request errors are redacted to
avoid leaking credentials or request details. Generated packages contain
controller identity and retrieval metadata only, never authentication material.

## GDSL resource limits

The GDSL collector uses sequential requests with a 5-second connection timeout
and a 15-second total request timeout. It reads at most 4 MiB plus one detection
byte from the response stream, rejecting larger bodies before parsing. GDSL
parsing emits at most 10,000 method symbols and accepts at most 256 parameters
per method. These limits apply before catalog validation and prevent an
unexpectedly large controller response from causing unbounded buffering or
symbol generation.

## Tests

    cargo fmt --manifest-path crates/jenkins-gdsl-collector/Cargo.toml -- --check
    cargo test --manifest-path crates/jenkins-gdsl-collector/Cargo.toml --all-targets
    cargo clippy --manifest-path crates/jenkins-gdsl-collector/Cargo.toml --all-targets -- -D warnings

## Deep Script Console discovery (explicit opt-in)

The `deep` profile first performs the same three bounded, sequential REST
requests as `quick`, then sends one repository-owned Groovy probe to the
authenticated `/scriptText` endpoint. It is never selected by default and is
not used by `quick`, `gdsl`, or ordinary Builder collection.

Inspect the exact request plan without contacting Jenkins:

```sh
cargo run --manifest-path crates/jenkins-gdsl-collector/Cargo.toml -- \
  https://jenkins.example.test/jenkins ./deep-plan.json --profile deep --dry-run
```

Only after reviewing the plan, explicitly confirm Script Console execution:

```sh
JENKINS_USERNAME=jenkins-admin JENKINS_API_TOKEN="$JENKINS_API_TOKEN" \
  cargo run --manifest-path crates/jenkins-gdsl-collector/Cargo.toml -- \
  https://jenkins.example.test/jenkins ./deep-catalog.json \
  --profile deep --confirm-script-console
```

The confirmation flag is mandatory for a real `deep` run. The probe source is
versioned in `src/probes/script_console_v1.groovy`; callers cannot supply
Groovy source. It reads only Groovy/JVM/Jenkins Core version metadata and the
registered Pipeline `StepDescriptor` and `GlobalVariable` extension lists.
It records item names, implementation class names, and plugin attribution when
available. Each registry is capped at 100 entries and each text field at 256
characters. The script also caps its JSON output; the client accepts at most
512 KiB and validates the probe/schema version before retaining any result.

The Script Console phase is one sequential POST request. It performs no
configuration writes, job/build execution, plugin installation, credential
access, filesystem scans, or network calls from Groovy. It does not enumerate
the ClassLoader or inspect arbitrary plugin classes. Missing extension types
and individual probe failures remain explicit in the runtime snapshot; the
collector does not replace them with static or inferred values. A denied
Script Console request is recorded as unavailable while preserving the
successful REST snapshot.

**Permissions and operational risk:** Jenkins Script Console access normally
requires the administrator-level `Overall/Administer` permission. Grant it
only to a dedicated, trusted account and only when this collection is
approved. Even a read-only Groovy script executes with the controller's
privileges; review the checked-in probe before running it. The client uses a
15-second request timeout and a bounded response, but an HTTP client timeout
does not guarantee that Jenkins has terminated a script already executing on
the controller. Run deep discovery deliberately, not as a scheduled or
background operation. Use HTTPS and inject credentials through an environment
or secret manager; credentials and script contents are never written to the
catalog or diagnostics.

The runtime record is stored separately under `controller.runtime`, with
schema version, template version, collection status, metadata, bounded
extension registries, and machine-readable partial-failure codes.
