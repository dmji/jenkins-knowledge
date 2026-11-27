# Controller discovery validation

This procedure separates offline contract evidence, the pinned live integration target, and an administrator-run smoke test. Fixture/contract CI does not contact an external Jenkins or Git remote. A dedicated CI job starts only the disposable, loopback-bound Jenkins 2.356 container documented in [the integration target](https://github.com/dmji/jenkins-knowledge/blob/master/tests/jenkins-2.356/README.md); it exercises the real HTTP, Git, and Script Console paths. No customer controller is contacted by CI. A separate live smoke test remains optional and must use a disposable or explicitly approved controller.

## Collection plan before a live run

Validate the profile and print the exact Builder request plan without making network calls:

```sh
cargo run --locked -p knowledge-builder -- plan profiles/production-pipelines.yml \
  > target/collection-plan.json
```

The command validates URLs, API roots, limits, package selectors, and the selected collection profile. `callsMade` is always `false`; credentials are never read or included. Inspect `networkRequests`, `sharedLibraries`, `requestCount`, `timeouts`, and `limits` before running `build`.

Per configured controller, Builder uses sequential HTTP requests:

| Profile | Requests | Controller operations |
| --- | ---: | --- |
| `quick` | 3 | Core metadata GET (1 MiB cap), plugin inventory GET (4 MiB cap), one-byte GDSL availability probe |
| `standard` | 4 | Quick operations plus full GDSL GET (4 MiB cap; at most 10,000 symbols and 256 parameters per symbol) |
| `deep` | 5 | Standard operations plus one POST to `/scriptText` (512 KiB cap) using the repository-owned fixed probe |

Each request has a 5-second connection timeout and a 15-second client timeout. Requests are sequential; there are no automatic retries. Shared Libraries are fetched through the configured Git transport and are listed separately from HTTP request counts. The deep Script Console plan includes the three extension registries (`pipeline-steps`, `pipeline-globals`, `declarative-extensions`) and the bounded API type inventory. Its default maxima are 40 types, depth 2, 12 methods per type, and 300 methods total; configured API roots are capped at 20. Client timeout does not guarantee that Jenkins terminates an already-running Script Console script.

For a standalone collector dry-run, which does not run Builder or fetch Shared Libraries:

```sh
cargo run --locked -p jenkins-gdsl-collector -- \
  https://jenkins.example.test/ ./target/deep-probe-plan.json \
  --profile deep --dry-run
```

This also reports `callsMade: false`. Builder's plan is authoritative for the complete profile, including the full GDSL download and any configured Shared Library remotes.

## Administrator smoke test

1. Create a copy of the production profile. Use one controller and only approved Shared Library remotes. Prefer HTTPS. Do not commit credentials or use URLs containing user information, query parameters, or fragments.
2. Start with `collectionProfile: quick`. Supply `JENKINS_USERNAME` and `JENKINS_API_TOKEN` through a secret manager or protected environment. The account needs only `Overall/Read`; plugin inventory and GDSL may be unavailable under the controller's authorization strategy. No plugin installation is required on the target Jenkins.
3. Generate and inspect the Builder plan. Confirm controller URL, endpoint paths, request count, response caps, library refs, and the absence of Script Console requests.
4. Run the quick build to a new output directory:

   ```sh
   cargo run --locked -p knowledge-builder -- build ./target/controller-smoke.yml ./target/controller-quick
   ```
5. If approved, change to `standard`, inspect a fresh plan, and build to a different output directory. This downloads GDSL and scans configured Shared Libraries; it does not execute Script Console.
6. Use `deep` only after separate security approval. Inspect the plan, review the checked-in `crates/jenkins-gdsl-collector/src/probes/script_console_v1.groovy`, grant a dedicated trusted account the required administrator permission for the duration of the run, and explicitly opt in by setting `collectionProfile: deep`; that profile selection is the Builder confirmation, and arbitrary caller-provided Groovy is never accepted. Do not schedule deep collection or treat its client timeout as server-side cancellation.
7. Inspect `manifest.json` collection outcomes and `catalog.json` source revisions, snapshot identity, coverage, confidence, and provenance. Keep the package local and access-controlled. Open the package read-only in Runtime/LSP/MCP; consumer startup and queries must not contact Jenkins or Git.

## Offline fixture report

The sanitized primary fixture used by the cross-component contract test contains **6 sources and 15 symbols**:

| Symbol kind | Count |
| --- | ---: |
| Pipeline steps | 5 |
| Declarative constructs | 3 |
| Groovy methods | 4 |
| Groovy classes | 2 |
| Shared Library globals | 1 |

Confidence distribution: 11 verified, 2 inferred, 2 unknown. Sources cover one controller snapshot, two plugin sources, a Declarative model, Jenkins Core, and a Shared Library. The fixture deliberately represents incomplete collection: plugin inventory permission denied, GDSL truncated, Script Console unavailable, and Shared Library scan failed. These outcomes remain visible; the fixture is not evidence of a successful live-controller run.

CI builds fixture packages for `quick`, `standard`, and `deep`, then validates the selected package through Runtime. A separate cross-component test exercises LSP and MCP against the deep fixture, including completion, hover, the verified unavailable-plugin diagnostic, collection status, symbol provenance, controller inventory, and profile isolation. A loopback listener verifies that Builder fixture builds and Runtime/LSP/MCP consumption do not contact a controller.

The separate `Jenkins 2.356 integration` Actions job runs the same three collection profiles against the digest-pinned `jenkins/jenkins:2.356-jdk17` image. It uses an ephemeral Matrix Authorization Strategy setup to verify Quick/Standard with `Overall/Read`, Deep with `Overall/Administer`, and Deep denial with the read-only account. It checks Core version, Java 17 metadata, plugin inventory, GDSL, a Shared Library pinned to a fixed commit, a deliberately unavailable Git revision, and profile-specific Script Console outcomes. Sanitized catalogs/manifests and the exact plugin inventory are uploaded as a workflow artifact. See [tests/jenkins-2.356](https://github.com/dmji/jenkins-knowledge/blob/master/tests/jenkins-2.356/README.md) for the pins and local reproduction command.

## Coverage boundaries and known gaps

- Quick discovery establishes controller identity/Core version, a bounded plugin inventory when permitted, and whether GDSL is available. It does not download GDSL or inspect runtime extension registries.
- Standard adds Pipeline GDSL and configured Shared Libraries. GDSL is an explicit DSL declaration source, not a complete Groovy grammar or proof of every plugin API.
- Deep adds only the fixed, bounded Script Console observations and explicitly configured API roots. It does not enumerate class loaders, scan plugin JARs, invoke arbitrary methods/constructors, or prove CPS/Sandbox safety.
- Shared Library scanning is a bounded declaration scanner, not a Groovy compiler. Dynamic metaprogramming, runtime-generated declarations, and unsupported syntax remain gaps and are reported rather than fabricated.
- Plugin inventory may be partial or permission-denied. A missing plugin/API record is not proof that it is absent.
- Controller-specific API roots are selected roots, not complete Jenkins Core or plugin API coverage. Reflection does not establish whether a method is safe to invoke from Pipeline code.
- The fixture's 14-symbol count is a deterministic test count only. Live counts vary by Jenkins Core version, installed plugins, permissions, GDSL output, configured API roots, and Shared Library revisions.

## Security checklist

- [ ] Use a disposable or explicitly approved controller; never run against production without approval.
- [ ] Use HTTPS and a dedicated service identity. Quick/standard need only the permissions required for Core, plugin inventory, and GDSL endpoints.
- [ ] Grant `Overall/Administer` for Script Console only for an explicitly approved deep run; do not broaden permissions merely to make optional inventory appear complete.
- [ ] Keep credentials in `JENKINS_USERNAME` / `JENKINS_API_TOKEN` or a secret manager. Never put them in profiles, command arguments, logs, fixture data, or package artifacts.
- [ ] Review the dry-run plan and fixed Groovy probe before execution. Do not accept caller-provided Groovy.
- [ ] Keep request limits, timeouts, sequential behavior, and explicit deep confirmation enabled. Do not add retries or background refresh.
- [ ] Store packages with restricted filesystem permissions; mount them read-only for Runtime/LSP/MCP and do not distribute raw internal controller metadata without review.
- [ ] Verify Runtime/LSP/MCP operate offline and do not hold collection credentials.