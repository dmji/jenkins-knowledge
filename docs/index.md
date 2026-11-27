# Jenkins Knowledge

**Turn an existing Jenkins controller into a versioned, locally consumable knowledge package.**

Jenkins Knowledge separates administrator-controlled collection from developer queries. Builder reads the configured Jenkins controller and optional Shared Libraries, then produces an immutable Knowledge Package. Runtime, MCP, and VS Code use that package locally — without Jenkins credentials, Git access, or background collection.

<div class="guide-summary">
  <div><strong>For administrators</strong><span>Collect and publish an evidence-backed package from an existing controller</span></div>
  <div><strong>For developers</strong><span>Search Pipeline APIs and library symbols from local tools</span></div>
  <div><strong>For operators</strong><span>Promote immutable releases and roll back without recollecting</span></div>
</div>

## Start with the end-to-end guide

The [deployment guide]({{ '/reference-deployment/guide.html' | relative_url }}) walks through the complete workflow in sequence: prepare the controller, configure access, collect and inspect the package, publish it, configure consumers, and operate future releases.

It assumes you already have a Jenkins controller. Installing or administering Jenkins is not a required part of the product lifecycle.

## Reference material

- [Architecture and trust boundaries]({{ '/architecture.html' | relative_url }}) — why collection and consumption are separate.
- [Catalog contract]({{ '/catalog-contract.html' | relative_url }}) — package schema, evidence, coverage, and completeness semantics.
- [Deployment acceptance checklist]({{ '/reference-deployment/acceptance-tests.html' | relative_url }}) — validate an integration before rollout.
- [Detailed deployment reference]({{ '/reference-deployment/index.html' | relative_url }}) — focused pages for individual operations.
- [Optional demo environment]({{ '/reference-deployment/demo-environment/walkthrough.html' | relative_url }}) — disposable Jenkins 2.356 / Java 17 fixture, not a production prerequisite.

## Product components

- [Builder source and commands](https://github.com/dmji/jenkins-knowledge/tree/master/crates/knowledge-builder)
- [Runtime and MCP](https://github.com/dmji/jenkins-knowledge/tree/master/crates/jenkins-runtime)
- [VS Code extension](https://github.com/dmji/jenkins-knowledge/tree/master/editors/vscode)

## Publishing

The site is built from `docs/` and deployed by the **Documentation Pages** GitHub Actions workflow. Pull requests validate the site without publishing it; merges to `master` trigger deployment.
