# Prebuilt Jenkins 2.356 package release policy

The prebuilt profile uses the `deep` collection profile against the digest-pinned Jenkins 2.356 / Java 17 reference image.

A package is accepted only when all of the following hold:

- Controller metadata and the complete plugin inventory are collected successfully.
- The observed Jenkins Core version, Java version/vendor, and every installed plugin version match the reference environment and its lock file.
- Pipeline GDSL and the pinned `gsa-examples` Shared Library source are collected successfully.
- Runtime opens the package, verifies its checksums, and returns results for the required Pipeline, plugin, and Shared Library smoke queries.
- No required source is unavailable, permission-denied, failed, or skipped.
- The package contains only `catalog.json` and `manifest.json`; cache, credentials, and temporary files remain outside the package.

## Explicitly accepted partial outcome

Jenkins 2.356 may report `script-console` as `partial` with `errorCode: enumeration-failed` when it cannot enumerate a subset of extension classes. This is the only accepted partial outcome for this reference profile. The package must still contain the observed Java version/vendor and the script-console runtime record; the partial status and error code remain visible in `manifest.json` and `catalog.json`.

A package with `collection.status: unknown`, any unavailable/failed/permission-denied required source, a partial source other than this specific Script Console enumeration outcome, or missing Java metadata is rejected. The workflow never rewrites a partial result as complete.

This policy describes the known behavior of the pinned reference environment. If the Jenkins image, Java version, plugin lock, or collector changes, the CI policy must be re-evaluated rather than silently broadening the allowlist.
