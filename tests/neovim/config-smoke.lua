-- Run with: nvim --headless -u NONE -l tests/neovim/config-smoke.lua
package.path = "lua/?.lua;" .. package.path

local setup = require("jenkins_knowledge").setup
setup({
  server_path = "/tmp/jenkins-lsp",
  package_path = "/tmp/jenkins-knowledge-package",
  profile = "smoke-test",
})

local config = vim.lsp.config.jenkins_knowledge
assert(config, "LSP config was not registered")
assert(config.cmd[1] == "/tmp/jenkins-lsp", "server executable was not configured")
assert(config.init_options.packagePath == vim.fn.fnamemodify(
  "/tmp/jenkins-knowledge-package", ":p"
), "package path was not passed to initialize")
assert(config.init_options.profile == "smoke-test", "profile was not passed to initialize")
assert(vim.lsp.is_enabled("jenkins_knowledge"), "LSP config was not enabled")
assert(vim.tbl_contains(config.filetypes, "groovy"), "Groovy filetype is not enabled")

assert(vim.filetype.match({ filename = "Jenkinsfile" }) == "groovy")
assert(vim.filetype.match({ filename = "Jenkinsfile.ci" }) == "groovy")
assert(vim.filetype.match({ filename = "build.jenkinsfile" }) == "groovy")

local workspace = vim.fn.tempname()
vim.fn.mkdir(workspace, "p")
local buffer = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_name(buffer, workspace .. "/vars/build.groovy")
local started = false
config.root_dir(buffer, function()
  started = true
end)
assert(not started, "ordinary Groovy files must not start Jenkins Knowledge")

vim.api.nvim_buf_set_name(buffer, workspace .. "/Jenkinsfile")
config.root_dir(buffer, function(root)
  assert(root == workspace, "Jenkinsfile should use its workspace directory")
  started = true
end)
assert(started, "Jenkinsfile should start Jenkins Knowledge")

vim.api.nvim_buf_delete(buffer, { force = true })
vim.fn.delete(workspace, "rf")
print("Neovim Jenkins Knowledge configuration smoke test passed")
vim.cmd("qa!")
