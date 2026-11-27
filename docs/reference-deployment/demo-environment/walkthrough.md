# Demo Environment: walkthrough

This walkthrough demonstrates the **custom Builder path** using a disposable local Jenkins controller. It is not required to use a prebuilt package. If you do not have a Jenkins controller to collect from, start with the [prebuilt package guide]({{ '/prebuilt-packages.html' | relative_url }}) and the [published Releases page](https://github.com/dmji/jenkins-knowledge/releases) instead.

Run from the repository root after reviewing [installation and safety]({{ '/reference-deployment/demo-environment/installation.html' | relative_url }}):

```sh
bash demo/up.sh
bash demo/build-package.sh
bash demo/verify.sh
bash demo/down.sh
```

The flow starts the disposable controller, collects a package, verifies Builder-to-Runtime/MCP/LSP handoff, and tears the environment down. Inspect generated output under the documented `target/` paths. Do not publish demo credentials or raw environment state.

The separate live integration and VS Code host acceptance scripts are described in [tests/jenkins-2.356](https://github.com/dmji/jenkins-knowledge/blob/master/tests/jenkins-2.356/README.md). Production integration does not require this demo.
