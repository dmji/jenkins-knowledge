# Backup and restore

## Backup

Back up the complete immutable package directory (`manifest.json`, `catalog.json`, and all files declared by the manifest checksums). Preserve bytes and directory structure. Back up profiles and non-secret operational configuration separately. The Builder snapshot cache is an optimization, not a backup. Keep secrets in a secret manager, never in profile backups.

## Restore

1. Retrieve the complete package artifact and verify its external checksum/signature if used.
2. Restore it to a new directory without editing files.
3. Run Runtime against the restored path to validate schemas and internal checksums.
4. Confirm package/profile and collection outcomes.
5. Point consumers to the restored path and run the [acceptance checklist](../acceptance-tests.md).

Missing files, checksum mismatches, invalid schemas, unsafe paths, or symlinks cause package loading to fail. Restore a known-good release rather than editing generated package files.
