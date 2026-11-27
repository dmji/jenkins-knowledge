# Refresh and scheduled operations

Knowledge refresh is an explicit Builder invocation. Runtime, MCP, and LSP do not poll Jenkins, contact Git, or refresh in the background.

1. Update the profile if controller URL, library revision, or package selection changed.
2. Inspect the network-free plan.
3. Build to a **new versioned output directory**.
4. Inspect manifest completeness and per-source outcomes.
5. Validate and smoke-test the package.
6. Publish through the approved artifact channel.
7. Roll consumers to the new package; retain the previous release for rollback.

```sh
knowledge-builder plan profiles/production-pipelines.yml
knowledge-builder refresh profiles/production-pipelines.yml ./out/production-pipelines-2026-12
```

Controller snapshots are cached locally under `${XDG_CACHE_HOME:-$HOME/.cache}/jenkins-knowledge/controller-snapshots` (or `JENKINS_KNOWLEDGE_CACHE_DIR`). Only fully successful snapshots are cached. `refresh` bypasses cache; `invalidate` removes selected controller cache entries. Cache is not a package or backup. Refresh does not overwrite existing package directories.
