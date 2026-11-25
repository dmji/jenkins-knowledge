# Jenkins Shared Library scanner

Scans configured Jenkins Shared Library Git repositories and emits one canonical catalog per library revision. It reads Groovy source from `vars/` and `src/`; it never executes library code.

## Configuration

Create a JSON array (for example `libraries.json`):

```json
[
  {
    "name": "platform",
    "repository": "https://github.com/example/platform-library.git",
    "reference": "refs/tags/v2.4.0"
  }
]
```

The reference may be a branch, tag, or commit accepted by `git fetch`. Git authentication uses the host's normal Git credential helper or environment; credentials must not be embedded in repository URLs.

## Run

```sh
cargo run --manifest-path crates/shared-library-scanner/Cargo.toml -- libraries.json catalog.json --diagnostics target/diagnostics.json
cargo test --manifest-path crates/shared-library-scanner/Cargo.toml
cargo fmt --manifest-path crates/shared-library-scanner/Cargo.toml -- --check
cargo clippy --manifest-path crates/shared-library-scanner/Cargo.toml --all-targets -- -D warnings
```

The catalog output is a JSON array containing one catalog for each configured library. Each catalog source records the configured ref in `version` and `metadata.configuredRef`, and the resolved 40-character Git commit in `revision` and `metadata.resolvedCommit`. Separate refs therefore produce separate source IDs and catalogs.

The optional `--diagnostics <path>` argument writes a versioned JSON object with a `schemaVersion` and a `diagnostics` array. Each diagnostic contains a stable `code`, `severity`, `sourceId`, repository-relative `path`, and human-readable `message`. The same findings continue to be printed as warnings to stderr. An empty scan writes an empty diagnostics array.

## Extraction model and limitations

- Each `vars/<name>.groovy` file contributes a `global-variable` symbol named after the file. An adjacent `vars/<name>.txt` file supplies its description when the script has no Groovydoc description; non-empty Groovydoc takes precedence, while its `@example` tags are retained.
- Explicit `call` and other method declarations are extracted from `vars/` scripts and `src/` classes. Parameter names, explicit types, optional defaults, overloads, preceding Groovydoc descriptions, and `@example` tags are retained when statically visible.
- Classes declared under `src/` are emitted as `class` symbols.
- Symbols include file and line provenance. Explicit declarations are marked `verified`; files containing dynamic dispatch/evaluation constructs are marked `inferred`.
- Dynamic dispatch, metaprogramming, generated methods, runtime inheritance and arbitrary Groovy expressions cannot be resolved without executing code. Such cases are reported in diagnostics rather than guessed. The scanner intentionally does not evaluate Groovy. Current diagnostic codes include `dynamic-groovy-constructs`, `dynamic-global-methods`, and `dynamic-method-dispatch`.
- The parser is a conservative source heuristic, not a complete Groovy grammar. Its supported declaration subset is:
  - class declarations with a simple identifier and same-line generic, `extends`, and `implements` clauses;
  - methods with explicit `def` or a typed/modifier-prefixed name, including annotations on preceding lines and parameter lists split across lines;
  - nested generic parameter types, nested collection/closure parameter types, default-valued parameters, and overloads;
  - simple class properties with an explicit type or `def`, including visibility and `static`/ `final` modifiers.
- Class members use the catalog `namespace` field for their declaring class; methods extracted from `vars/<name>.groovy` use the global name as their namespace. Every extracted declaration retains its repository-relative file and line in `provenance`.
- Unsupported declaration forms currently include traits, enums, records, annotation declarations, sealed classes, closure-parameter declarations, and incomplete method signatures. They produce `unsupported-groovy-syntax` diagnostics and do not create guessed symbols.
- This is intentionally not a complete Groovy parser: multiline class headers, nested/local classes, complex field initializers, script-generated members, extension methods, AST transforms, and dynamic metaprogramming may not be represented. An absent symbol is not evidence that a member does not exist. Treat diagnostics as coverage warnings, not compiler errors.

The scanner validates each generated catalog against `schemas/catalog.schema.json` and the semantic source-reference checks from `catalog-validator`.


## Safety and resource limits

Only absolute HTTPS/SSH remotes and Git SCP-style remotes are accepted. Local paths, `file://`, `git://`, embedded HTTPS credentials, query strings, fragments, and unsafe Git refs are rejected by the same validation policy in Builder and the standalone scanner.

Git fetches are shallow, omit tags, request blobs no larger than 2 MiB, disable terminal prompts, and each Git subprocess is terminated after 120 seconds. Git server implementations may ignore partial-clone filters, so traversal limits remain the final local safety boundary.

The scanner skips symlinks (including directory symlinks), resolves every candidate against the canonical checkout root, and enforces these defaults:

| Limit | Default | Environment override |
|---|---:|---|
| Traversal depth | 32 directories | `JENKINS_KNOWLEDGE_SCAN_MAX_DEPTH` |
| Groovy files | 10,000 | `JENKINS_KNOWLEDGE_SCAN_MAX_FILES` |
| Individual Groovy file | 1 MiB | `JENKINS_KNOWLEDGE_SCAN_MAX_FILE_BYTES` |
| Aggregate Groovy source | 32 MiB | `JENKINS_KNOWLEDGE_SCAN_MAX_TOTAL_BYTES` |
| Adjacent text documentation | 1 MiB per file | fixed |

Overrides must be positive integers. Exceeding a limit fails the scan with a diagnostic naming the exceeded limit; it does not silently truncate results. These limits apply to source traversal; Git subprocess timeouts and shallow filtered fetches bound the preceding checkout phase.
