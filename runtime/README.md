# Runtime

The Runtime consumes a built, immutable Knowledge Package. It must not need access to Jenkins controllers or source Git repositories.

## Planned interfaces

Read-only operations:

- `search_api` — find symbols by name, kind, source, or profile.
- `get_api_details` — retrieve signature, parameters, return type, provenance, and confidence.
- `list_api` — enumerate symbols within a scope.
- `get_usage_examples` — return catalog-backed examples.
- `get_pipeline_step` — retrieve Jenkins Pipeline step metadata.

Planned MCP resources:

- `jenkins://catalog/manifest`
- `jenkins://catalog/libraries`
- `jenkins://catalog/pipeline-steps`
- `jenkins://catalog/profiles/{profile}`

The Runtime is retrieval-only. It does not execute Pipeline steps or trigger Jenkins jobs.\n\nThe Rust implementation and CLI live in `crates/jenkins-runtime`. A package directory contains `manifest.json` and its catalog file. The CLI validates both package and catalog schemas, then supports `search`, `list`, `get`, `examples`, and `step` commands. See the root README for commands and filters.
