# Inspect and publish a Knowledge Package

A build creates `manifest.json` and `catalog.json`. The manifest records package/schema/tool versions, build timestamp, source revisions, checksums, and collection outcomes. The catalog contains normalized symbols, provenance, controller snapshot, and optional coverage/runtime metadata.

Runtime verifies declared files with SHA-256 before parsing and rejects missing/mismatched files, malformed or duplicate checksum entries, unsafe paths, symlinks, and schema violations. The manifest is not self-checksummed.

## Completeness

A successful build does not mean complete collection. Optional unavailable, truncated, permission-denied, or failed sources make the package partial. Unknown and not-collected are not success. Manifest outcomes describe collection execution; catalog coverage describes observed API coverage. An absent symbol is not proof that Jenkins lacks that API: collection is bounded/profile-dependent, and reflection does not prove CPS or Script Security Sandbox compatibility.

## Publish

Treat each package as an immutable release. Validate it, calculate an external checksum if required by your artifact channel, retain its profile/source revisions, and transfer the complete directory unchanged. Do not distribute Builder credentials or cache contents. Mount packages read-only for consumers.
