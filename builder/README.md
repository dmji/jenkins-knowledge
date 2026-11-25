# Builder

The Builder produces a deterministic Knowledge Package from configured sources.

## Planned pipeline

1. Resolve and record exact source revisions.
2. Scan Shared Libraries (starting with `vars/`, then `src/` and resources).
3. Import controller-specific Pipeline DSL metadata from Jenkins, including GDSL where available.
4. Collect plugin/core metadata and pipeline step descriptors.
5. Normalize symbols into the canonical catalog model.
6. Validate the catalog against `schemas/catalog.schema.json`.
7. Export `catalog.json`, `manifest.json`, generated artifacts, and checksums.

## Security

- Credentials are injected at build time through environment variables or a secret provider.
- Never serialize credentials into profiles, catalogs, logs, or package metadata.
- Store source URLs and immutable revisions for reproducibility.

Implementation has not started; this document describes the intended contract.
