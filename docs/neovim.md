---
title: Neovim 0.12.6
---

# Neovim 0.12.6 integration

Jenkins Knowledge provides a small Lua setup module for Neovim's built-in LSP client. It uses the native Neovim 0.12.6 APIs—vim.lsp.config, vim.lsp.enable, and vim.pack—and does not require nvim-lspconfig or another LSP plugin.

The Lua module configures the existing jenkins-lsp executable. The executable and the Knowledge Package are separate inputs:

- jenkins-lsp comes from the platform tool bundle under bin/jenkins-lsp.
- The Knowledge Package is collected from the user's Jenkins instance by knowledge-builder and contains manifest.json and catalog.json.
- The Neovim module is installed from this repository and does not contain package data or Jenkins credentials.

## Requirements

- Neovim 0.12.6 or newer with the built-in vim.pack and native LSP APIs.
- Git for vim.pack's initial plugin checkout.
- The jenkins-lsp executable for the host OS/architecture. Prefer the verified platform tool bundle from the [Jenkins Knowledge releases](https://github.com/dmji/jenkins-knowledge/releases). The archive contains bin/jenkins-lsp.
- A Knowledge Package directory collected from the intended Jenkins controller. The profile, Core/plugin inventory, and Shared Library revisions must be the ones the developer needs to query.

The LSP executable reads one package at startup. It does not connect to Jenkins/Git, run a Pipeline, or refresh collection data. The Builder and package lifecycle are described in the [deployment guide]({{ '/reference-deployment/guide.html' | relative_url }}).

## Stock Neovim: install with vim.pack

Add the repository to the built-in package manager, then call the module's setup function. The integration module is at the repository root under lua/jenkins_knowledge, so vim.pack can load it directly.

~~~lua
vim.pack.add({
  {
    src = "https://github.com/dmji/jenkins-knowledge",
    version = "master",
  },
})

require("jenkins_knowledge").setup({
  server_path = vim.fn.expand("~/.local/share/jenkins-knowledge/tools/bin/jenkins-lsp"),
  package_path = vim.fn.expand("~/jenkins-knowledge/production-pipelines"),
  profile = "production-pipelines",
})
~~~

Replace the two paths and profile with the local values. server_path may also be the command name jenkins-lsp when it is on PATH. package_path may be absolute or relative to the Jenkinsfile workspace root; an absolute path is easiest to reason about. The optional profile must exactly match the profile in the package manifest.

The repository's Neovim module currently lives on the master branch; the older v0.0.1 release tag predates this integration. vim.pack's lockfile records the checked-out revision. Commit the generated lockfile with your Neovim configuration if you want the plugin revision to be reproducible.

## LazyVim

LazyVim uses its normal plugin specification. The same setup module is loaded from the repository; the LSP itself remains the bundled executable.

Create lua/plugins/jenkins-knowledge.lua:

~~~lua
return {
  {
    "dmji/jenkins-knowledge",
    branch = "master",
    lazy = false,
    config = function()
      require("jenkins_knowledge").setup({
        server_path = vim.fn.expand("~/.local/share/jenkins-knowledge/tools/bin/jenkins-lsp"),
        package_path = vim.fn.expand("~/jenkins-knowledge/production-pipelines"),
        profile = "production-pipelines",
      })
    end,
  },
}
~~~

lazy = false ensures the module registers Jenkinsfile filetype detection during startup, before a Jenkinsfile is opened. No nvim-lspconfig dependency is needed; the module defines the server directly with vim.lsp.config and enables it with vim.lsp.enable.

## Supported files and scope

The module registers these names as Groovy and restricts LSP startup to them:

| Filename | LSP behavior |
|---|---|
| Jenkinsfile | Attach |
| Jenkinsfile.* | Attach |
| *.jenkinsfile | Attach |
| Ordinary *.groovy files | Do not attach |
| vars/*.groovy and src/**/*.groovy Shared Library sources | Do not attach directly |

Shared Library symbols collected into the package can still appear in Jenkinsfile completion and hover. The current LSP server does not provide direct editor features inside Shared Library source files; the Neovim integration deliberately does not pretend otherwise.

For supported Jenkinsfiles, the server provides:

- **Completion:** Scripted Pipeline steps, globals, Shared Library symbols, and context-supported Jenkins Core/plugin types. Declarative directives are offered outside steps; script blocks use Scripted/Groovy scope.
- **Hover:** exact-name overloads, parameter/return information, plugin availability, source revision/provenance, execution constraints, confidence, and package profile.
- **Go to Definition:** only when safe, revision-pinned source evidence exists.
- **Diagnostics:** verified Declarative-context mistakes, unavailable plugin symbols, CPS incompatibility, and Script Security Sandbox rejection. Unknown evidence is not turned into an error.

The setup module enables Neovim's built-in LSP completion with automatic trigger characters. Inside a Pipeline step block, use CTRL-X CTRL-O to request completion manually; typing a dot can also trigger completion. Press K over a symbol for hover. For definition navigation, run:

~~~vim
:lua vim.lsp.buf.definition()
~~~

Diagnostics use Neovim's built-in vim.diagnostic display. They are conservative evidence-based findings, not a complete Groovy compiler or Pipeline execution validator.

## Verify the connection

Open a supported Jenkinsfile and check that the client attached:

~~~vim
:checkhealth vim.lsp
:lua print(vim.inspect(vim.lsp.get_clients({ bufnr = 0, name = "jenkins_knowledge" })))
~~~

Then exercise a real catalog feature:

1. In a Pipeline steps block, invoke CTRL-X CTRL-O and confirm Jenkins Knowledge completions are returned from the selected package.
2. Place the cursor on a known step or Shared Library symbol and press K to inspect the package-backed hover.
3. If the symbol has a safe source location, invoke vim.lsp.buf.definition() and confirm navigation targets the recorded revision.

The repository's Neovim 0.12.6 smoke test verifies the setup module, executable argument handling, filetype detection, Jenkinsfile-only attachment filter, initialization options, and completion activation. Existing LSP integration tests cover the server's protocol and package behavior.

## Troubleshooting

| Symptom | Check |
|---|---|
| No LSP client attaches | Confirm the buffer name is Jenkinsfile, Jenkinsfile.* or *.jenkinsfile and its filetype is groovy. Ordinary Groovy and Shared Library source files are intentionally excluded. |
| Server does not start | Check server_path, executable permission, OS/CPU architecture, and run :checkhealth vim.lsp. The LSP process takes no --package argument. |
| No catalog results | Confirm package_path points to a complete package directory containing manifest.json and catalog.json. |
| Profile mismatch | Remove the optional profile setting or set it to the exact profile recorded in manifest.json. |
| Old package data remains | Restart the LSP client after replacing the package. The server loads one package per process; it does not watch the directory. |
| No completion popup | Confirm the LSP client is attached, then invoke CTRL-X CTRL-O manually inside a supported Jenkinsfile context. Completion is limited to symbols present in the selected package. |
| No definition target | The symbol may not have a safe revision-pinned source location. The server intentionally returns no invented target. |

## Implementation references

- [Neovim integration module](https://github.com/dmji/jenkins-knowledge/blob/master/lua/jenkins_knowledge/init.lua)
- [Neovim config smoke test](https://github.com/dmji/jenkins-knowledge/blob/master/editors/neovim/tests/config_spec.lua)
- [Neovim 0.12.6 package-manager documentation](https://github.com/neovim/neovim/blob/v0.12.6/runtime/doc/pack.txt)
- [Neovim 0.12.6 LSP documentation](https://github.com/neovim/neovim/blob/v0.12.6/runtime/doc/lsp.txt)
- [LSP server implementation](https://github.com/dmji/jenkins-knowledge/blob/master/crates/jenkins-lsp/src/main.rs)
