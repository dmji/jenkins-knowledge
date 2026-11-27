# Prebuilt Knowledge Package releases

A prebuilt package is a convenience download for one explicitly identified reference environment. It lets a Runtime user start querying Jenkins knowledge without running collection against their own controller. It does not replace custom collection when the target Jenkins version, Java runtime, or plugin set differs.

## Published reference package

Version tags matching `vMAJOR.MINOR.PATCH` (optionally with a prerelease suffix) trigger the same live Jenkins 2.356 / Java 17 deep collection and validation workflow used on pull requests. A GitHub Release is created only after the package build and every required CI check succeeds, and only when the tag points to a commit already reachable from `master`.

Each release contains two assets:

- `jenkins-knowledge-vX.Y.Z-jenkins-2.356-java17-deep.tar.gz` — the complete Knowledge Package directory.
- The same filename with `.sha256` appended — SHA-256 checksum of the compressed archive.

The release description records the exact Jenkins Core and Java versions, complete plugin inventory, collection profile and status, accepted partial outcome if present, package/catalog schema versions, Builder tool versions, source revisions, and source commit. The archive is not modified after the package's internal checksum validation.

A release tag is immutable for this publishing workflow. If a GitHub Release already exists for a tag, the workflow fails rather than replacing or silently overwriting assets. Publish a new version tag for a new package.

## Download, verify, and extract

Set the release tag you want, then download both assets:

```sh
TAG=vX.Y.Z
ASSET="jenkins-knowledge-${TAG}-jenkins-2.356-java17-deep.tar.gz"
BASE_URL="https://github.com/dmji/jenkins-knowledge/releases/download/${TAG}"

curl -fLO "${BASE_URL}/${ASSET}"
curl -fLO "${BASE_URL}/${ASSET}.sha256"
```

On Linux, verify the archive before extracting it:

```sh
sha256sum -c "${ASSET}.sha256"
tar -xzf "${ASSET}"
```

On macOS, use `shasum -a 256 -c "${ASSET}.sha256"` for the checksum step. Keep the checksum file and archive in the same directory so the filename recorded in the checksum file resolves correctly.

The extracted directory is `prebuilt-package/`, containing `manifest.json` and `catalog.json`. Runtime validates the manifest, catalog schema, and internal catalog checksum when it opens the package.

## Use with Runtime and MCP

Point the CLI at the extracted directory:

```sh
jenkins-runtime --package ./prebuilt-package plugins
jenkins-runtime --package ./prebuilt-package steps
jenkins-runtime --package ./prebuilt-package search deploy
```

Start the read-only MCP server with the same package path:

```sh
jenkins-mcp --package ./prebuilt-package
```

Configure LSP or the VS Code extension to use this same local package path, following the [VS Code setup guide](../editors/vscode/README.md). Runtime, MCP, and LSP do not download releases, contact Jenkins or Git, or update the package in the background. Selecting a different package is an explicit user action.

## Scope and completeness

This prebuilt package targets exactly Jenkins 2.356, Java 17, and the plugin versions listed in the corresponding release notes and catalog. It is not a compatibility guarantee for another controller, even if the Jenkins Core version appears similar. Differences in plugin versions, plugin configuration, Java, or controller APIs can change the collected knowledge.

The reference collector has one known bounded limitation: Jenkins 2.356 can report `script-console` as `partial` with `enumeration-failed` when a subset of extension classes cannot be enumerated. The workflow accepts only this specific partial outcome, while requiring controller metadata, the complete plugin inventory, Pipeline GDSL, the pinned Shared Library source, and Java metadata to succeed. The package metadata preserves the partial status; it is never rewritten as complete. Any other failed, unavailable, or partial required source blocks release publication.

For another Jenkins/Java/plugin combination, build a custom Knowledge Package from that controller using the [Builder documentation](../crates/knowledge-builder/README.md). The Runtime format is shared between prebuilt and custom packages.
