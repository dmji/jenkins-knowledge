# Cross-component contract tests

The `jenkins-contract-tests` crate builds sanitized fixture packages through the real Builder path, then consumes the same package through Runtime, the LSP stdio server, and the MCP stdio server.

The primary fixture includes controller metadata, plugin inventory, GDSL steps, a partial Script Console extension registry, Declarative descriptors, Jenkins Core/Groovy type metadata, and Shared Library symbols. Sanitized raw response fixtures for `/api/json`, `/pluginManager/api/json`, and `/scriptText` live under `crates/jenkins-gdsl-collector/tests/fixtures/controller/`; the GDSL fixture is kept separately. Its adjacent `manifest.json` supplies explicit per-surface collection outcomes so Builder can preserve unavailable, permission-denied, truncated, and failed optional surfaces. Fixture catalogs without an adjacent manifest retain the existing all-succeeded fixture behavior.

The offline profile test builds fixture packages with `quick`, `standard`, and `deep` collection profiles and checks the expected surface boundaries: quick retains only Declarative symbols, standard adds GDSL and Shared Library symbols, and deep adds controller-observed API types. The deep fixture contains 15 symbols (11 verified, 2 inferred, 2 unknown) across six sources; its collection manifest deliberately includes permission-denied, truncated, unavailable, and failed outcomes.

The integration test verifies:

- Identical symbol IDs, source IDs, revisions, confidence, and evidence through Runtime, LSP, and MCP.
- Partial collection metadata remains visible to all consumers.
- Duplicate names and overloaded methods remain distinguishable; unknown confidence is not upgraded.
- Type/member search, LSP completion/hover, and MCP tool/resource discovery work against the built package.
- LSP package/profile switching drops results from the previous package.
- Consumer queries leave package bytes unchanged and do not connect to the fixture controller endpoint.

The test starts the production LSP and MCP stdio entrypoints in child processes. The fixture endpoint is a loopback listener owned by the test; no live Jenkins instance, credentials, Git remote, Script Console execution, or external network access is required.

CI runs this suite in the dedicated **Cross-component offline contract tests** job. The broader workspace validation excludes this crate to avoid running the same end-to-end suite twice.