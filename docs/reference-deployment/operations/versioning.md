# Package versioning and promotion

`profile` and optional `package.id` identify the logical selection, not a unique release. Record each build as an immutable artifact release.

Retain package ID/profile, collection profile, manifest timestamp, Builder/schema versions, Jenkins Core/plugin inventory and outcomes, resolved Shared Library commit SHAs, completeness, external checksum, publication location, validation result, and consumer rollout reference.

Build every release to a new output directory. Promote the same tested artifact between environments when the source snapshot is unchanged. If a source changes, collect a new package and retain the previous one for rollback. Matching version strings alone do not prove API/Pipeline compatibility; the offline compatibility report is deliberately conservative.
