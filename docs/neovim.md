---
title: Neovim integration
---

# Neovim 0.12.6 and LazyVim

Jenkins Knowledge provides a small Lua setup module for the existing `jenkins-lsp` executable. The module configures the native Neovim LSP client; it does not implement catalog lookup in Lua. The LSP reads a local Knowledge Package and does not connect to Jenkins.

## Prerequisites

- Neovim **0.12.6**.
- The `jenkins-lsp` executable from the matching [Jenkins Knowledge tool bundle](https://github.com/dmji/jenkins-knowledge/releases).
- A built Knowledge Package directory containing `manifest.json`. Use a package collected from the target Jenkins controller for controller-specific assistance.
- An absolute path to the executable if it is not on `PATH`, and an absolute package path.

The server currently advertises text synchronization, completion (triggered by `.`), hover, and go-to-definition. Jenkins catalog features are limited to files named `Jenkinsfile`, `Jenkinsfile.*`, or `*.jenkinsfile`; ordinary Groovy source files are not treated as Jenkinsfiles.

## Stock Neovim with the built-in package manager

Add the repository using Neovim 0.12's native `vim.pack` manager. The repository root contains the reusable `lua/jenkins_knowledge.lua` module.

In `init.lua`:

```lua
vim.pack.add({
  { src = "https://github.com/dmji/jenkins-knowledge" },
})

require("jenkins_knowledge").setup({
  server_path = "/opt/jenkins-knowledge/bin/jenkins-lsp",
  package_path = "/srv/jenkins-knowledge/production-pipelines",
  profile = "production-pipelines",
})
```

If `jenkins-lsp` is available on `PATH`, `server_path` may be omitted. `profile` is optional; when supplied, it must match the profile recorded in the package.

The module registers Jenkinsfile filename patterns as the `groovy` filetype, configures `vim.lsp.config`, and enables the `jenkins_knowledge` client. Its root-directory callback filters out ordinary Groovy files before starting the server.

## LazyVim

Create `lua/plugins/jenkins-knowledge.lua` in your LazyVim configuration:

```lua
return {
  {
    "dmji/jenkins-knowledge",
    ft = "groovy",
    config = function()
      require("jenkins_knowledge").setup({
        server_path = "/opt/jenkins-knowledge/bin/jenkins-lsp",
        package_path = "/srv/jenkins-knowledge/production-pipelines",
        profile = "production-pipelines",
      })
    end,
  },
}
```

Both installation paths call the same `setup()` function and use the same Rust LSP server. LazyVim is not a prerequisite for stock Neovim.

## Verify the integration

Run the headless configuration smoke test from the repository root with Neovim 0.12.6:

```sh
nvim --headless -u NONE -l tests/neovim/config-smoke.lua
```

Then perform an end-to-end check against a real package:

1. Start Neovim 0.12.6 and open a Jenkinsfile.
2. Run `:set filetype?`; it should report `filetype=groovy`.
3. Run `:checkhealth vim.lsp` and confirm `jenkins_knowledge` is enabled.
4. Run `:lua =vim.lsp.get_clients({ name = "jenkins_knowledge", bufnr = 0 })` and confirm that a client is attached.
5. Type a known Pipeline step followed by a dot to trigger completion, hover a known symbol, and use the editor's definition action on a symbol with source-location evidence.

Open an ordinary `.groovy` file and confirm that the Jenkins Knowledge client does not attach.

## Troubleshooting

- **No client starts:** run `jenkins-lsp` from a terminal to verify that the executable can start; check `server_path` and execute permissions.
- **Package cannot be opened:** confirm that `package_path` is an absolute path to a built package directory containing `manifest.json`.
- **Profile mismatch:** remove the optional `profile` setting or set it to the exact package profile.
- **No completion or hover:** confirm the filename is `Jenkinsfile`, `Jenkinsfile.*`, or ends in `.jenkinsfile`; confirm the LSP client is attached and the selected package contains the requested symbol.
- **Wrong filetype:** inspect `:set filetype?`; the module registers the supported Jenkinsfile naming patterns as Groovy.

See [LSP and MCP interfaces]({{ '/interfaces.html' | relative_url }}) for the server's implemented capabilities and protocol details.
