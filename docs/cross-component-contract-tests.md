# Cross-component contract tests

The `jenkins-contract-tests` crate builds sanitized fixture packages through the real Builder path, then consumes the same package through Runtime, the LSP stdio server, and the MCP stdio server.

The primary fixture includes controller metadata, plugin inventory, GDSL steps, a partial Script Console extension registry, Declarative descriptors, Jenkins Core/Groovy type metadata, and Shared Library symbols. Its adjacent `manifest.json` supplies explicit per-surface collection outcomes so Builder can preserve unavailable, permission-denied, truncated, and failed optional surfaces. Fixture catalogs without an adjacent manifest retain the existing all-succeeded fixture behavior.

The integration test verifies:

- Identical symbol IDs, source IDs, revisions, confidence, and evidence through Runtime, LSP, and MCP.
- Partial collection metadata remains visible to all consumers.
- Duplicate names and overloaded methods remain distinguishable; unknown confidence is not upgraded.
- Type/member search, LSP completion/hover, and MCP tool/resource discovery work against the built package.
- LSP package/profile switching drops results from the previous package.
- Consumer queries leave package bytes unchanged and do not connect to the fixture controller endpoint.

The test starts the production LSP and MCP stdio entrypoints in child processes. The fixture endpoint is a loopback listener owned by the test; no live Jenkins instance, credentials, Git remote, Script Console execution, or external network access is required.

CI runs this suite in the dedicated **Cross-component offline contract tests** job. The broader workspace validation excludes this crate to avoid running the same end-to-end suite twice.
