local root = vim.fn.getcwd()
package.path = root .. "/lua/?.lua;" .. root .. "/lua/?/init.lua;" .. package.path

local temp = vim.fn.tempname()
local project = temp .. "/project"
local git_dir = project .. "/.git"
vim.fn.mkdir(git_dir, "p")

local server_path = temp .. "/fake jenkins lsp"
vim.fn.mkdir(temp, "p")
vim.fn.writefile({ "#!/bin/sh", "exit 0" }, server_path)
vim.fn.setfperm(server_path, "rwxr-xr-x")
assert(vim.fn.executable(server_path) == 1, "test LSP executable must be executable")

local original_enable = vim.lsp.enable
local enabled
vim.lsp.enable = function(name)
  enabled = name
end

local package_path = "~/Jenkins Knowledge/package"
local config = require("jenkins_knowledge").setup({
  server_path = server_path,
  package_path = package_path,
  profile = "production-pipelines",
})

vim.lsp.enable = original_enable

assert(enabled == "jenkins_knowledge")
assert(config.cmd[1] == server_path, "executable paths with spaces must remain a single argv item")
assert(config.filetypes[1] == "groovy")
assert(config.init_options.packagePath == vim.fn.expand(package_path))
assert(config.init_options.profile == "production-pipelines")

assert(vim.filetype.match({ filename = "Jenkinsfile" }) == "groovy")
assert(vim.filetype.match({ filename = "Jenkinsfile.groovy" }) == "groovy")
assert(vim.filetype.match({ filename = "release.jenkinsfile" }) == "groovy")

local supported = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_name(supported, project .. "/Jenkinsfile")
local resolved_root
config.root_dir(supported, function(value)
  resolved_root = value
end)
assert(resolved_root == project, "Jenkinsfile should use the nearest Git root")

local unsupported = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_name(unsupported, project .. "/vars/deployApp.groovy")
local unsupported_attached = false
config.root_dir(unsupported, function()
  unsupported_attached = true
end)
assert(not unsupported_attached, "ordinary/Shared Library Groovy files must not attach")

local original_completion_enable = vim.lsp.completion.enable
local completion_args
vim.lsp.completion.enable = function(...)
  completion_args = { ... }
end
config.on_attach({
  id = 42,
  supports_method = function(_, method, bufnr)
    return method == "textDocument/completion" and bufnr == supported
  end,
}, supported)
vim.lsp.completion.enable = original_completion_enable

assert(completion_args and completion_args[1] == true)
assert(completion_args[2] == 42 and completion_args[3] == supported)
assert(completion_args[4].autotrigger == true)

local ok, err = pcall(function()
  require("jenkins_knowledge").setup({ server_path = server_path })
end)
assert(not ok and tostring(err):match("package_path is required"))

vim.api.nvim_buf_delete(supported, { force = true })
vim.api.nvim_buf_delete(unsupported, { force = true })
vim.fn.delete(temp, "rf")
print("Jenkins Knowledge Neovim config smoke passed")
