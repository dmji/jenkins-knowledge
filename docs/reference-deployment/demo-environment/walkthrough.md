# Demo Environment: walkthrough

Run from the repository root after reviewing [installation and safety](installation.md):

```sh
bash demo/up.sh
bash demo/build-package.sh
bash demo/verify.sh
bash demo/down.sh
```

The flow starts the disposable controller, collects a package, verifies Builder-to-Runtime/MCP/LSP handoff, and tears the environment down. Inspect generated output under the documented `target/` paths. Do not publish demo credentials or raw environment state.

The separate live integration and VS Code host acceptance scripts are described in [tests/jenkins-2.356](../../../../tests/jenkins-2.356/README.md). Production integration does not require this demo.
