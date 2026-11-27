# Jenkins Knowledge

**Build or consume a versioned Jenkins Knowledge Package — with or without administering Jenkins.**

Jenkins Knowledge separates administrator-controlled collection from developer queries. Builder reads the configured Jenkins controller and optional Shared Libraries, then produces an immutable Knowledge Package. Runtime, MCP, and VS Code use that package locally — without Jenkins credentials, Git access, or background collection.

<div class="guide-summary">
  <div><strong>For administrators</strong><span>Collect and publish an evidence-backed package from an existing controller</span></div>
  <div><strong>For developers</strong><span>Search Pipeline APIs and library symbols from local tools</span></div>
  <div><strong>For operators</strong><span>Promote immutable releases and roll back without recollecting</span></div>
</div>

## Choose your starting point

- **No Jenkins controller to collect from?** Start with a [published prebuilt package]({{ '/prebuilt-packages.html' | relative_url }}), then follow the download, checksum, extraction, and Runtime/MCP/LSP steps. Browse the [GitHub Releases page](https://github.com/dmji/jenkins-knowledge/releases); versioned assets appear after the first release tag is published.
- **Need knowledge from your own Jenkins?** Follow the [custom deployment guide]({{ '/reference-deployment/guide.html' | relative_url }}). An administrator configures Builder for the real controller, Java runtime, plugin inventory, and Shared Libraries; developers consume the resulting package locally.

Both paths produce the same package contract and use the same offline Runtime. The [optional local demo]({{ '/reference-deployment/demo-environment/walkthrough.html' | relative_url }}) is a disposable Builder example, not a prerequisite for the prebuilt path.

## Reference material

- [Architecture and trust boundaries]({{ '/architecture.html' | relative_url }}) — why collection and consumption are separate.
- [Catalog contract]({{ '/catalog-contract.html' | relative_url }}) — package schema, evidence, coverage, and completeness semantics.
- [Prebuilt package releases]({{ '/prebuilt-packages.html' | relative_url }}) — download, verify, extract, and use the versioned Jenkins 2.356 reference package.
- [Deployment acceptance checklist]({{ '/reference-deployment/acceptance-tests.html' | relative_url }}) — validate an integration before rollout.
- [Detailed deployment reference]({{ '/reference-deployment/index.html' | relative_url }}) — focused pages for individual operations.
- [Optional demo environment]({{ '/reference-deployment/demo-environment/walkthrough.html' | relative_url }}) — disposable Jenkins 2.356 / Java 17 fixture, not a production prerequisite.

## Product components

- [Builder source and commands](https://github.com/dmji/jenkins-knowledge/tree/master/crates/knowledge-builder)
- [Runtime and MCP](https://github.com/dmji/jenkins-knowledge/tree/master/crates/jenkins-runtime)
- [VS Code extension](https://github.com/dmji/jenkins-knowledge/tree/master/editors/vscode)

## Publishing

The site is built from `docs/` and deployed by the **Documentation Pages** GitHub Actions workflow. Pull requests validate the site without publishing it; merges to `master` trigger deployment.
