# Choose and operate a Knowledge Package

Jenkins Knowledge supports two package acquisition paths. They produce the same Knowledge Package format and are consumed by the same offline Runtime, MCP server, and LSP.

- **Prebuilt reference package:** for someone who does not administer a Jenkins controller and wants to start querying immediately. Download a published release, verify it, and select the extracted directory.
- **Custom package:** for an administrator who needs knowledge collected from their own Jenkins controller and its actual Java/plugin environment. Run Builder against that controller, then distribute the resulting package to developers.

A prebuilt package is a convenience snapshot for one declared reference environment. It does not replace collection from a different Jenkins installation.

## 1. Prebuilt reference package

### Available releases

Published assets are listed on the [Jenkins Knowledge GitHub Releases page](https://github.com/dmji/jenkins-knowledge/releases). Each versioned release contains the portable Knowledge Package plus ready-to-run platform tool bundles for Linux x86_64, macOS Apple Silicon, macOS Intel, and Windows x86_64. Each bundle contains knowledge-builder, jenkins-runtime, jenkins-mcp, jenkins-lsp, and groovy-stubs; the executables are separate from the package data.

The current reference profile is Jenkins 2.356 / Java 17 with the exact plugin versions recorded in the release catalog. The asset name includes both the product release version and the reference environment identifier:

- `jenkins-knowledge-vX.Y.Z-jenkins-2.356-java17-deep.tar.gz` (reference package)
- `jenkins-knowledge-tools-vX.Y.Z-linux-x86_64.tar.gz` (or the matching platform suffix)
- The same names with `.sha256` appended.

The release description is generated from the package and records the exact Jenkins Core and Java versions, complete plugin inventory, collection profile/status, accepted partial outcome, package/catalog schema versions, Builder tool versions, source revisions, build time, and source commit.

### Download, verify, and extract

After choosing a version on the [Releases page](https://github.com/dmji/jenkins-knowledge/releases), set that exact tag and download both assets:

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

On macOS, use `shasum -a 256 -c "${ASSET}.sha256"` instead of `sha256sum`. Keep the checksum file and archive in the same directory: the checksum records the archive filename, not an absolute path.

The extracted directory is `prebuilt-package/`, containing `manifest.json` and `catalog.json`. Runtime validates the manifest, catalog schema, and internal catalog checksum when it opens the package. The external checksum verifies the complete compressed archive; the internal manifest checksum verifies the catalog.

### Download the matching platform tool bundle

Choose the archive for your operating system and CPU architecture from the same GitHub Release. The examples below use Linux x86_64; use the corresponding `linux-x86_64`, `macos-aarch64`, `macos-x86_64`, or `windows-x86_64` asset on other hosts.

```sh
TAG=vX.Y.Z
PLATFORM=linux-x86_64
TOOLS_ARCHIVE="jenkins-knowledge-tools-${TAG}-${PLATFORM}.tar.gz"
BASE_URL="https://github.com/dmji/jenkins-knowledge/releases/download/${TAG}"
curl -fLO "${BASE_URL}/${TOOLS_ARCHIVE}"
curl -fLO "${BASE_URL}/${TOOLS_ARCHIVE}.sha256"
sha256sum -c "${TOOLS_ARCHIVE}.sha256"
mkdir -p tools && tar -xzf "${TOOLS_ARCHIVE}" -C tools
RUNTIME="$PWD/tools/bin/jenkins-runtime"
BUILDER="$PWD/tools/bin/knowledge-builder"
MCP="$PWD/tools/bin/jenkins-mcp"
LSP="$PWD/tools/bin/jenkins-lsp"
STUBS="$PWD/tools/bin/groovy-stubs"

"$RUNTIME" --package ./prebuilt-package plugins
"$RUNTIME" --package ./prebuilt-package steps
"$RUNTIME" --package ./prebuilt-package search deploy
"$RUNTIME" --package ./prebuilt-package compatibility
"$RUNTIME" --package ./prebuilt-package coverage
```

On macOS, use `shasum -a 256 -c` instead of `sha256sum`. On Windows, extract the `.tar.gz` archive with a compatible archiver and invoke the `.exe` files under `tools\bin\` from PowerShell. No Rust toolchain, Cargo, compiler, or source checkout is required to run the bundled executables.

The `compatibility` command without a target reports the environment recorded by the package and returns `not-compared` / `unknown`. Runtime is offline and does not infer a Jenkins installation from the machine on which it runs.

### Use with MCP

Launch the downloaded MCP stdio server with the same package. It reads JSON-RPC over standard input/output and does not expose a network listener:

```sh
"$PWD/runtime/bin/jenkins-mcp" --package "$PWD/prebuilt-package"
```

For OpenCode, use the current `mcp.servers` configuration with a local command array:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "jenkins_knowledge": {
        "type": "local",
        "command": ["/absolute/path/to/jenkins-mcp", "--package", "/absolute/path/to/prebuilt-package"],
        "protocol": "legacy",
        "codemode": false
      }
    }
  }
}
```

See the [complete OpenCode setup and MCP contract]({{ '/interfaces.html' | relative_url }}).

MCP exposes the same read-only search/details APIs and catalog resources. The `jenkins://catalog/compatibility` resource returns the offline report without a target. The `get_package_compatibility` tool can compare against a caller-supplied target environment; see [MCP configuration and contract]({{ '/reference-deployment/integration/mcp.html' | relative_url }}).

### Use with LSP / VS Code

The bundle includes the `jenkins-lsp` executable. Configure the VS Code extension to use the same local package:

```json
{
  "jenkinsKnowledge.serverPath": "/absolute/path/to/tools/bin/jenkins-lsp",
  "jenkinsKnowledge.packagePath": "/absolute/path/to/prebuilt-package",
  "jenkinsKnowledge.profile": "jenkins-2356-prebuilt",
  "files.associations": {
    "Jenkinsfile": "groovy",
    "*.jenkinsfile": "groovy"
  }
}
```

The configured profile must match the package manifest. The LSP custom requests `jenkinsKnowledge/collection` and `jenkinsKnowledge/coverage` expose collection outcomes and API-surface coverage separately. The LSP does not infer compatibility with a local Jenkins installation.

Runtime, MCP, and LSP do not download releases, contact Jenkins or Git, or update the package in the background. Selecting a different package is an explicit user action. The Runtime release version is independent of the Knowledge Package version; opening a package performs embedded manifest/catalog schema, metadata, and checksum validation. This establishes package-format compatibility only, not compatibility with a different Jenkins controller or plugin environment.

## 2. Build a custom package from your Jenkins

Use this path when the package must reflect your own controller, its actual Java runtime, installed plugin versions, configured Shared Libraries, and API surfaces. A release built for the reference environment is not a substitute for this collection.

1. Copy or edit a Builder profile for your controller. Set the real controller URL, desired collection profile, and any Shared Library repositories/revisions. Pin library commits to full commit SHAs where reproducibility is required.
2. Inspect the plan without network access:

   ```sh
   cargo run --locked --manifest-path crates/knowledge-builder/Cargo.toml -- plan profiles/production-pipelines.yml
   ```

3. Supply credentials through the process environment or a secret manager; do not put tokens in profile URLs or commit them to the repository. For a controller requiring authentication, Builder expects both `JENKINS_USERNAME` and `JENKINS_API_TOKEN`:

   ```sh
   export JENKINS_USERNAME="your-jenkins-user"
   export JENKINS_API_TOKEN="your-jenkins-api-token"
   cargo run --locked --manifest-path crates/knowledge-builder/Cargo.toml -- build profiles/production-pipelines.yml ./target/custom-package
   unset JENKINS_USERNAME JENKINS_API_TOKEN
   ```

4. Inspect the generated `manifest.json`, `catalog.json`, source outcomes, and plugin inventory. Use Runtime to query the package exactly as you would query a prebuilt package:

   ```sh
   cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package ./target/custom-package plugins
   cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package ./target/custom-package steps
   cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package ./target/custom-package compatibility
   ```

The profile's `collectionProfile` controls depth. `quick` and `standard` avoid the Deep Script Console probe; Java version/vendor may therefore be recorded as `not-collected-by-profile`. `deep` runs the fixed read-only Script Console probe and is needed when those JVM properties and deeper API metadata are required. See the [Builder collection profile and credential reference](https://github.com/dmji/jenkins-knowledge/blob/master/crates/knowledge-builder/README.md) before choosing.

Distribute the resulting package directory as an immutable versioned artifact using your organization's release process. The package contract is the same as the prebuilt release; developers do not need Builder credentials or network access to Jenkins/Git.

## 3. Read the manifest, inventory, and completeness

The package separates structural validity, collection completeness, API-surface coverage, and environment comparison. Do not treat them as one compatibility flag.

- **Manifest:** records package and catalog schema versions, package profile, build time, tool versions, source revisions, checksums, and the package-level collection result.
- **Collection status:** `complete` or `partial`; per-source outcomes explain what succeeded, was skipped, was unavailable, or failed. Older packages without the collection object are `unknown`, not complete.
- **Plugin inventory:** the catalog records each installed plugin's short name and version for the captured controller. This is the observed inventory, not a promise that every plugin API was fully understood.
- **Java metadata:** the controller record includes Java version/vendor and separate status fields. A missing Java value can mean the selected collection profile did not run the Script Console probe; it must not be inferred from the Jenkins version.
- **API coverage:** Runtime's `coverage` command reports inspected API surface. Coverage is distinct from package collection completeness.
- **Checksums:** Runtime verifies every declared package file before using the catalog. The release's external SHA-256 verifies the archive before extraction.

The current reference collector has one known bounded limitation: Jenkins 2.356 can report `script-console` as `partial` with `enumeration-failed` when a subset of extension classes cannot be enumerated. The release workflow accepts only this specific partial outcome, while requiring controller metadata, the complete plugin inventory, Pipeline GDSL, the pinned Shared Library source, and Java metadata to succeed. The package metadata preserves the partial status; it is never rewritten as complete. Any other failed, unavailable, or partial required source blocks release publication.

## 4. Understand the compatibility report

Run:

```sh
cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package ./prebuilt-package compatibility
```

Without a target environment, the report describes only the package's recorded environment and collection limitations. Its comparison status is `not-compared`; compatibility is `unknown`. This is expected for an offline package.

For an explicit comparison, use MCP's `get_package_compatibility` tool and supply a `targetEnvironment` object containing:

- `coreVersion`
- `javaVersion`
- `javaVendor`
- a complete `plugins` array with `shortName` and `version` for every installed plugin.

Interpret the result conservatively:

| Result | Meaning |
|---|---|
| `environment-different` | One or more recorded versions differ. This is a known difference, not proof the package cannot run. |
| `not-proven` | The supplied target matches the recorded Core, Java, and plugin inventory and package completeness is complete. Matching versions still do not prove API, configuration, or Pipeline compatibility. |
| `unknown` | Target metadata is incomplete, package metadata is missing, or collection completeness is partial/unknown. |
| `not-compared` | No target environment was supplied. |

A compatibility report is evidence for an explicit comparison, not an automatic compatibility certification. Differences do not always make a package unusable; matching versions do not prove full compatibility. For the current Jenkins 2.356 reference package, the documented Script Console partial outcome means the package completeness assessment remains unknown even when version fields match.

## 5. Update the reference package

A new Jenkins Core version, Java version, plugin lock, or reference profile is a new reference environment snapshot. Do not overwrite an existing Release or silently replace its assets.

Maintainers update the pinned Jenkins image digest, profile, plugin lock, reference environment identifier, validation expectations, and asset naming together. CI then rebuilds and independently validates the package. After those changes are merged to `master`, publishing a new `vX.Y.Z` tag triggers a new GitHub Release with a new archive and checksum. The product release tag is independent from `packageVersion` and `catalogSchemaVersion`; publishing a new reference environment does not by itself require a schema-version change. The release job rejects tags that do not point to a commit reachable from `master` and refuses to overwrite an existing Release.

Maintainers publish from the merged `master` commit:

```sh
git fetch origin master
git switch master
git pull --ff-only
git tag -a vX.Y.Z -m "Jenkins Knowledge vX.Y.Z"
git push origin vX.Y.Z
```

Consumers remain pinned to the package directory they selected. To adopt a newer package, download a new release, verify its checksum, extract it, review its manifest/release notes, and explicitly update the Runtime/MCP/LSP package path. No background update occurs.

## 6. Optional local Jenkins demo

The [local demo walkthrough]({{ '/reference-deployment/demo-environment/walkthrough.html' | relative_url }}) is a separate Builder example. It starts a disposable Jenkins 2.356 controller, collects a package, verifies Builder-to-Runtime/MCP/LSP handoff, and tears the environment down. It is useful for learning the custom collection path, but it is not required to use a prebuilt package and is not a production deployment recipe.
