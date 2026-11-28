local M = {}

local function is_jenkinsfile(bufnr)
  local name = vim.fs.basename(vim.api.nvim_buf_get_name(bufnr))
  return name == "Jenkinsfile"
    or name:match("^Jenkinsfile%.") ~= nil
    or name:match("%.jenkinsfile$") ~= nil
end

---@class JenkinsKnowledgeOptions
---@field server_path? string Path or executable name for jenkins-lsp
---@field package_path string Absolute path to a built Knowledge Package directory
---@field profile? string Expected package profile

---@param opts JenkinsKnowledgeOptions
function M.setup(opts)
  opts = opts or {}
  assert(type(opts.package_path) == "string" and opts.package_path ~= "",
    "jenkins_knowledge.setup: package_path is required")

  vim.filetype.add({
    filename = { Jenkinsfile = "groovy" },
    pattern = {
      ["Jenkinsfile%..+"] = "groovy",
      [".*%.jenkinsfile"] = "groovy",
    },
  })

  local package_path = vim.fn.fnamemodify(opts.package_path, ":p")
  local server_path = opts.server_path or "jenkins-lsp"

  vim.lsp.config("jenkins_knowledge", {
    cmd = { server_path },
    filetypes = { "groovy" },
    init_options = {
      packagePath = package_path,
      profile = opts.profile,
    },
    root_dir = function(bufnr, on_dir)
      if not is_jenkinsfile(bufnr) then
        return
      end
      local filename = vim.api.nvim_buf_get_name(bufnr)
      local root = vim.fs.root(bufnr, { ".git" }) or vim.fs.dirname(filename)
      on_dir(root)
    end,
  })

  vim.lsp.enable("jenkins_knowledge")
end

return M
