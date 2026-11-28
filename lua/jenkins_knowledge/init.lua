local M = {}

local SERVER_NAME = "jenkins_knowledge"

local function is_supported_filename(name)
  return name == "Jenkinsfile"
    or name:match("^Jenkinsfile%.") ~= nil
    or name:match("%.jenkinsfile$") ~= nil
end

function M.setup(opts)
  opts = opts or {}

  local server_path = opts.server_path or "jenkins-lsp"
  if type(server_path) ~= "string" or server_path == "" then
    error("jenkins_knowledge: server_path must be a non-empty executable path", 2)
  end
  server_path = vim.fn.expand(server_path)
  if vim.fn.executable(server_path) ~= 1 then
    error(("jenkins_knowledge: LSP executable is not executable: %s"):format(server_path), 2)
  end

  local package_path = opts.package_path
  if type(package_path) ~= "string" or package_path == "" then
    error("jenkins_knowledge: package_path is required", 2)
  end
  package_path = vim.fn.expand(package_path)

  local profile = opts.profile
  if profile ~= nil and (type(profile) ~= "string" or profile == "") then
    error("jenkins_knowledge: profile must be a non-empty string when provided", 2)
  end

  vim.filetype.add({
    filename = {
      Jenkinsfile = "groovy",
    },
    pattern = {
      ["Jenkinsfile%..*"] = "groovy",
      [".*%.jenkinsfile"] = "groovy",
    },
  })

  local config = {
    cmd = { server_path },
    filetypes = { "groovy" },
    root_dir = function(bufnr, on_dir)
      local path = vim.api.nvim_buf_get_name(bufnr)
      local basename = vim.fs.basename(path)
      if not is_supported_filename(basename) then
        return
      end

      local root = vim.fs.root(bufnr, { ".git" }) or vim.fs.dirname(path)
      if root then
        on_dir(root)
      end
    end,
    init_options = {
      packagePath = package_path,
      profile = profile,
    },
    on_attach = function(client, bufnr)
      if client:supports_method("textDocument/completion", bufnr) then
        vim.lsp.completion.enable(true, client.id, bufnr, { autotrigger = true })
      end
    end,
  }

  vim.lsp.config(SERVER_NAME, config)
  vim.lsp.enable(SERVER_NAME)

  return config
end

return M
