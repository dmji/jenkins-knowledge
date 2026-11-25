# Jenkins GDSL collector

Collects the controller-specific Pipeline DSL from Jenkins' authenticated
`/pipeline-syntax/gdsl` endpoint and emits a catalog conforming to schema 1.0.0.

## Usage

```sh
JENKINS_USERNAME=ci-reader JENKINS_API_TOKEN="$JENKINS_API_TOKEN" \
  cargo run --manifest-path crates/jenkins-gdsl-collector/Cargo.toml -- \
  https://jenkins.example.test/jenkins ./catalog.json
```

For anonymous/read-only endpoints, omit both environment variables. The URL must
be HTTP(S) and must not contain user information. Credentials are sent as HTTP
Basic authentication, are never written to output or diagnostics, and should be
provided through a secret manager/environment injection. Use HTTPS in production.

## Compatibility and evidence

The collector targets Jenkins installations exposing the Pipeline Syntax GDSL
endpoint (Pipeline: Groovy / workflow-cps plugin family). GDSL is Groovy DSL
source, not XML. The parser extracts explicit `method(name: ...)` declarations;
it does not execute Groovy, evaluate dynamic expressions, or infer missing APIs.
Parameter descriptions, return types, and documentation URLs are retained only
when explicitly represented in the declaration. Jenkins version is recorded from
the `X-Jenkins` response header when present. Plugin-specific steps are included
when the installed controller's GDSL exposes them.

Each symbol is marked `verified` because its name is backed by an explicit
controller declaration. Provenance identifies the endpoint and source line.
The source revision records retrieval time because GDSL does not expose a stable
content revision. A controller snapshot should therefore be regenerated when
Jenkins or its plugins change.

HTTP 401/403, other non-success responses, empty responses, invalid UTF-8,
invalid URLs, oversized responses, timeouts, and malformed declarations produce
structured errors. Request errors are redacted to avoid leaking credentials or
request details. The generated package contains controller identity and retrieval
metadata only, never authentication material.

## Resource limits

The collector uses sequential requests with a 5-second connection timeout and a
15-second total request timeout. It reads at most 4 MiB plus one detection byte
from the response stream, rejecting larger bodies before parsing. GDSL parsing
emits at most 10,000 method symbols and accepts at most 256 parameters per method.
These limits apply before catalog validation and prevent an unexpectedly large
controller response from causing unbounded buffering or symbol generation.

## Tests

```sh
cargo fmt --manifest-path crates/jenkins-gdsl-collector/Cargo.toml -- --check
cargo test --manifest-path crates/jenkins-gdsl-collector/Cargo.toml --all-targets
cargo clippy --manifest-path crates/jenkins-gdsl-collector/Cargo.toml --all-targets -- -D warnings
```
