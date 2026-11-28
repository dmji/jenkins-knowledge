---
title: End-to-end deployment guide
---

# From Jenkins controller to developer workflow

This is the primary implementation guide for Jenkins Knowledge. It follows one deployment from an existing Jenkins controller to the developer workstation, then explains how to keep that deployment current. Work through the stages in order; each stage produces an artifact or decision needed by the next one.

The production path does **not** install Jenkins, require Docker, or give developer tools access to the controller. An administrator collects knowledge once, publishes an immutable package, and distributes that package to local consumers.

<div class="guide-summary">
  <div><strong>Starting point</strong><span>An existing Jenkins controller and an administrator-operated Builder host</span></div>
  <div><strong>Deliverable</strong><span>A versioned Knowledge Package with explicit source coverage and provenance</span></div>
  <div><strong>Consumers</strong><span>Runtime CLI, MCP server, and VS Code — all local and read-only</span></div>
</div>

## The deployment in one view

1. **Prepare** — confirm compatibility, identify one controller, and choose the collection profile.
2. **Connect** — create a dedicated read-oriented identity, provide credentials at runtime, and configure optional Shared Libraries.
3. **Collect** — review the plan, run Builder, and understand which sources were actually collected.
4. **Publish** — validate the package, preserve it as an immutable release, and transfer it to consumer hosts.
5. **Connect consumers** — configure Runtime, MCP, and VS Code to use that exact package.
6. **Operate** — refresh into a new release, promote tested artifacts, and retain a rollback path.

The boundary between stages matters: only Builder needs Jenkins/Git access and collection credentials. Runtime, MCP, and LSP must not collect, refresh, or mutate knowledge.

## 1. Prepare the collection

### Check the environment

Builder runs on an administrator-controlled host with the pinned Rust toolchain and Git available. It needs network access only to the Jenkins controller and the explicitly configured Shared Library remotes. Docker is not part of ordinary collection.

The package describes the controller that was actually inspected. Record its Jenkins Core version and plugin inventory; do not infer Java version from Core. Java vendor/version are observed only by the opt-in Deep Script Console probe. Quick and Standard intentionally leave them uncollected.

Compatibility is evidence-based. It depends on Core and plugin versions, permissions, GDSL availability, library revisions, and the selected profile. The Jenkins 2.356 / Java 17 integration target is a tested fixture, not a minimum or production recommendation. See the [tested integration matrix](https://github.com/dmji/jenkins-knowledge/blob/master/tests/jenkins-2.356/README.md).

### Choose the collection depth

| Profile | What it collects | When to use it |
| --- | --- | --- |
| **Quick** | Required controller metadata, optional plugin inventory, and a GDSL availability probe. No full GDSL, library scan, or Script Console. | Initial connectivity check or fast snapshot |
| **Standard** | Quick's controller/inventory data, full GDSL extraction, and configured Shared Libraries. No Script Console. | Normal production collection |
| **Deep** | Standard plus a fixed, bounded, read-only Script Console probe and explicitly configured API roots. | Only when deeper runtime/API evidence is needed and approved |

Start with Standard unless there is a specific reason to choose otherwise. Deep is not a general-purpose shell: it executes a repository-owned bounded read-only probe and requires explicit security approval.

## 2. Connect the existing controller

Create a profile for the controller you intend to document. A package currently supports one controller snapshot; use separate profiles and packages for separate controllers.

```yaml
profile: production-pipelines
collectionProfile: standard
jenkins:
  controllers:
    - id: production
      url: https://jenkins.example.com
libraries: []
package:
  id: production-pipelines
```

Use the real HTTPS origin and context path. Do not put credentials, query parameters, or fragments in the URL. The profile is an administrator-controlled allowlist, not a place for end-user supplied endpoints.

Before collection, inspect the plan. This is a local, network-free review of configured endpoints, request budgets, limits, Script Console probe details, and Git remotes:

```sh
knowledge-builder plan profiles/production-pipelines.yml
```

### Use a dedicated identity

Use a dedicated service identity rather than a personal administrator account. The live integration test exercises Quick and Standard with a read-oriented identity granted `Overall/Read`, `Item/Read`, and `View/Read`. Actual requirements can vary with Jenkins security configuration and plugins; begin with least privilege and use source outcomes to identify a specific missing permission.

Do not broaden permissions merely to make optional inventory complete. Plugin inventory may legitimately remain unknown or unavailable.

Deep is different: the Script Console probe is tested with `Overall/Administer`. Do not grant that privilege to the routine collection identity. If Deep is approved, use a separately controlled identity and treat the resulting package as privileged operational output.

### Supply credentials at runtime

Builder reads `JENKINS_USERNAME` and `JENKINS_API_TOKEN`. Supply both or neither. For an interactive run:

```sh
read -r -p "Jenkins username: " JENKINS_USERNAME
export JENKINS_USERNAME
read -r -s -p "Jenkins API token: " JENKINS_API_TOKEN
printf '\\n'
export JENKINS_API_TOKEN

/opt/jenkins-knowledge/tools/bin/knowledge-builder build profiles/production-pipelines.yml ./out/production-pipelines-2026-11

unset JENKINS_USERNAME JENKINS_API_TOKEN
```

In automation, inject them from the CI secret store at job runtime. Never commit tokens, place them in profile URLs or command arguments, bake them into images, or copy them into the package. Git access for Shared Libraries should use the Builder host's credential helper or SSH agent.

### Add Shared Libraries when they are part of the controller's API

If Pipeline behavior depends on Global Variables or methods from Shared Libraries, declare those repositories in the profile:

```yaml
libraries:
  - name: platform-lib
    url: https://git.example.com/platform/platform-lib.git
    revision: release/2.x
    commit: 0123456789abcdef0123456789abcdef01234567
```

Replace the illustrative SHA with the real full 40-character commit. Pinning makes collection reproducible. If `commit` is omitted, Builder resolves the configured ref and records the resolved revision. Quick intentionally skips libraries; Standard and Deep scan configured libraries independently. The scanner is bounded declaration analysis, not a Groovy compiler, so dynamic metaprogramming may not be discovered.

## 3. Collect and inspect the Knowledge Package

Run the build only after reviewing the plan and confirming access:

```sh
knowledge-builder build profiles/production-pipelines.yml ./out/production-pipelines-2026-11
```

The output directory must not already exist. Build each release into a new versioned directory; Builder does not overwrite an existing package.

A successful process exit does not necessarily mean complete coverage. Optional sources can be unavailable, permission-denied, failed, truncated, or skipped by the selected profile. Those outcomes are recorded rather than silently converted into empty success. In particular:

- Controller identity and Core metadata are mandatory. If they fail, collection aborts.
- Plugin inventory is optional; unknown inventory does not prove that no plugins are installed.
- GDSL coverage depends on the selected profile and controller/plugin behavior.
- Shared Library failures are isolated and recorded as partial outcomes.
- Script Console data exists only when Deep was selected and its probe was permitted.

The package contains `manifest.json` and `catalog.json`. The manifest records tool/schema versions, build time, source revisions, checksums, and collection outcomes. The catalog contains normalized symbols, provenance, controller snapshot, and optional coverage/runtime metadata.

Read the manifest before publishing. Distinguish **what the collector attempted** (manifest outcomes) from **what API surface was observed** (catalog coverage). An absent symbol is not proof that Jenkins lacks that API: collection is bounded and profile-dependent, and reflection does not prove CPS or Script Security Sandbox compatibility.

## 4. Validate and publish an immutable release

Treat the complete package directory as one immutable artifact. Runtime verifies declared files with SHA-256 before parsing and rejects missing or mismatched files, malformed checksum entries, unsafe paths, symlinks, and schema violations. The manifest itself is not self-checksummed.

Before transfer:

1. Confirm the package ID, collection profile, Core/plugin snapshot, and resolved library commits.
2. Review every source outcome and the overall completeness status.
3. Run Runtime against the package as a smoke test.
4. Calculate an external checksum if required by your artifact channel.
5. Transfer the complete directory unchanged and retain its provenance.

Do not distribute Builder credentials or local snapshot-cache contents. Mount the package read-only on consumer hosts.

## 5. Configure local consumers

Consumers need the package and their executable, not access to Jenkins or Git. The platform tool bundle provides knowledge-builder, jenkins-runtime, jenkins-mcp, jenkins-lsp, and groovy-stubs for Linux x86_64, macOS Apple Silicon/Intel, and Windows x86_64. Download and verify the bundle as described in [prebuilt packages]({{ '/prebuilt-packages.html' | relative_url }}). The commands below use the extracted tools/bin directory. Use the [LSP and MCP interface guide]({{ '/interfaces.html' | relative_url }}) for the complete feature reference, editor setup, MCP schemas, and OpenCode configuration.

### Runtime CLI

Runtime provides local search, listing, exact symbol lookup, examples, and Pipeline-step inspection:

```sh
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines search deploy
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines list --kind step
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines get deployApp
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines examples deployApp
```

Use the release executable in production rather than invoking Cargo for each query. The package is required unless `JENKINS_KNOWLEDGE_PACKAGE` is set. If you specify `--profile`, it must match the manifest.

### MCP server

The MCP server exposes one validated package over stdio. One process serves exactly one package/profile:

Example OpenCode configuration (adjust absolute paths):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "jenkins_knowledge": {
        "type": "local",
        "command": ["/opt/jenkins-knowledge/tools/bin/jenkins-mcp", "--package", "/srv/jenkins-knowledge/production-pipelines"],
        "protocol": "legacy",
        "codemode": false
      }
    }
  }
}
```

The server exposes read-only search/list, symbol details, examples, Pipeline steps, controller inventory, compatibility information, and package resources. It does not execute Pipeline steps or connect to Jenkins/Git.

### VS Code extension

Use jenkins-lsp from the tool bundle and build the repository's VS Code extension, then point the workspace settings at the local executable and package:

```sh
cd editors/vscode
npm ci
npm run compile
```

```json
{
  "jenkinsKnowledge.serverPath": "/opt/jenkins-knowledge/tools/bin/jenkins-lsp",
  "jenkinsKnowledge.packagePath": "/srv/jenkins-knowledge/production-pipelines",
  "jenkinsKnowledge.profile": "production-pipelines",
  "files.associations": {
    "Jenkinsfile": "groovy",
    "*.jenkinsfile": "groovy"
  }
}
```

Jenkins-specific completion, hover, navigation, and evidence-based diagnostics apply to Jenkinsfile documents; ordinary Groovy remains with installed Groovy tooling. The configured profile must match the package manifest. After publishing a new package, use **Jenkins Knowledge: Reload Knowledge Package**. Changing settings does not invoke Builder or refresh the source data.

At this point, validate the workflow from the consumer side: Runtime returns expected symbols, MCP starts with the same package, and VS Code provides Jenkinsfile completion/hover. Confirm all three work without Jenkins/Git network access or collection credentials.

## 6. Keep the deployment current

Knowledge refresh is an explicit Builder operation. Runtime, MCP, LSP, and groovy-stubs do not poll Jenkins, contact Git, or refresh in the background.

For every update, repeat the collection and publication stages into a **new output directory**:

1. Update the profile if controller URL, library revision, or package selection changed.
2. Review the network-free plan.
3. Build the new package.
4. Inspect completeness and per-source outcomes.
5. Validate and smoke-test the artifact.
6. Publish it through the approved artifact channel.
7. Roll consumers forward only after validation; retain the previous release for rollback.

Record the package ID/profile, collection profile, build timestamp, Builder/schema versions, Core/plugin inventory and outcomes, resolved library commits, external checksum, publication location, validation result, and consumer rollout reference. Promote the same tested artifact between environments when its source snapshot has not changed. A changed source requires a new collection.

The Builder snapshot cache is only an optimization; it is not part of the package or a backup. `refresh` bypasses cache, while `invalidate` removes selected controller cache entries. Neither operation overwrites an existing package directory.

## Recovery and acceptance

Back up the complete package directory, preserving every file and its bytes. Keep profiles and non-secret operational configuration separately; keep credentials in a secret manager. To restore, retrieve the artifact, verify its external checksum if used, restore it unchanged to a new directory, and run Runtime against it before switching consumers.

Use the [deployment acceptance checklist]({{ '/reference-deployment/acceptance-tests.html' | relative_url }}) before rollout and after restore. For failures, the [troubleshooting reference]({{ '/reference-deployment/operations/troubleshooting.html' | relative_url }}) maps common symptoms to source outcomes and corrective actions.

## Optional: reproduce the flow in a demo environment

The disposable Jenkins 2.356 / Java 17 demo is useful for evaluation and integration testing. It is not a prerequisite for production deployment and is not a production installation recipe. Follow the [demo walkthrough]({{ '/reference-deployment/demo-environment/walkthrough.html' | relative_url }}) only when you need the local fixture.

## Detailed reference

The focused pages remain available for operational details and exact contracts:

- [Collection prerequisites]({{ '/reference-deployment/prerequisites.html' | relative_url }})
- [Architecture and trust boundaries]({{ '/architecture.html' | relative_url }})
- [Catalog contract]({{ '/catalog-contract.html' | relative_url }})
- [Acceptance tests]({{ '/reference-deployment/acceptance-tests.html' | relative_url }})

---

*The key operational rule: Builder collects; the Knowledge Package carries the evidence; Runtime, MCP, and VS Code consume that package locally and read-only.*
