# Deployment acceptance checklist

## Sources and security

- [ ] Profile identifies exactly one intended controller and URL contains no credentials/query/fragment.
- [ ] Dedicated Builder identity has only required permissions.
- [ ] API token injected at runtime from secret storage; not committed or passed as CLI argument.
- [ ] Shared Library remotes are reachable; releases pin full 40-character SHAs.
- [ ] Network-free plan reviewed before collection.
- [ ] Deep/Script Console has explicit security approval if selected.

## Collection and package

- [ ] Quick/Standard/Deep selection matches required coverage.
- [ ] Mandatory Core metadata succeeded.
- [ ] Plugin, GDSL, library, and Script Console outcomes reviewed individually.
- [ ] Partial/unavailable/denied/failed/truncated/skipped/unknown are not mislabeled complete.
- [ ] Core/plugin versions and resolved library commits recorded.
- [ ] Java metadata is not inferred; marked uncollected unless Deep observed it.
- [ ] Package schemas/checksums validate and artifact is immutable.

## Consumers and operations

- [ ] Runtime loads expected package/symbols.
- [ ] MCP exposes expected read-only tools/resources.
- [ ] VS Code LSP path/package/profile configured; Jenkinsfile completion/hover work.
- [ ] Ordinary Groovy does not receive Jenkins-specific completion/hover.
- [ ] Runtime/MCP/LSP work without Jenkins/Git network access or collection credentials.
- [ ] Consumer package mount is read-only.
- [ ] Previous release retained; refresh uses new output path; backup/restore tested.
