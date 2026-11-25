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
cargo run --manifest-path crates/shared-library-scanner/Cargo.toml -- libraries.json catalog.json
cargo test --manifest-path crates/shared-library-scanner/Cargo.toml
cargo fmt --manifest-path crates/shared-library-scanner/Cargo.toml -- --check
cargo clippy --manifest-path crates/shared-library-scanner/Cargo.toml --all-targets -- -D warnings
```

The output is a JSON array containing one catalog for each configured library. Each catalog source records the configured ref in `version` and `metadata.configuredRef`, and the resolved 40-character Git commit in `revision` and `metadata.resolvedCommit`. Separate refs therefore produce separate source IDs and catalogs.

## Extraction model and limitations

- Each `vars/<name>.groovy` file contributes a `global-variable` symbol named after the file.
- Explicit `call` and other method declarations are extracted from `vars/` scripts and `src/` classes. Parameter names, explicit types, optional defaults, overloads, preceding Groovydoc descriptions, and `@example` tags are retained when statically visible.
- Classes declared under `src/` are emitted as `class` symbols.
- Symbols include file and line provenance. Explicit declarations are marked `verified`; files containing dynamic dispatch/evaluation constructs are marked `inferred`.
- Dynamic dispatch, metaprogramming, generated methods, runtime inheritance and arbitrary Groovy expressions cannot be resolved without executing code. Such cases are reported in diagnostics rather than guessed. The scanner intentionally does not evaluate Groovy.
- The parser is a conservative source heuristic, not a complete Groovy grammar. Unsupported syntax should be treated as a diagnostic, not proof that an API does not exist.

The scanner validates each generated catalog against `schemas/catalog.schema.json` and the semantic source-reference checks from `catalog-validator`.
