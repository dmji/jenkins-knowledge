---
title: LSP and MCP interfaces
---

# Use Jenkins Knowledge from editors and AI agents

Jenkins Knowledge exposes one selected Knowledge Package through two complementary interfaces:

- **Language Server Protocol (LSP)** gives an editor contextual assistance while a developer writes a Jenkinsfile.
- **Model Context Protocol (MCP)** gives an AI agent or MCP-compatible client structured access to the same catalog.

Both interfaces use the read-only Runtime. Neither connects to Jenkins or Git, runs a Pipeline, refreshes a package, or substitutes for the administrative Builder. Builder collects the target controller and configured Shared Libraries; the package records the Core/plugin snapshot and evidence that the two interfaces expose.

Use LSP for inline authoring assistance. Use MCP when an agent needs to search, inspect, compare, and explain catalog data. OpenCode consumes MCP; it does not automatically use the LSP integration.

## Choose an interface

| Task | LSP in an editor | MCP in an agent |
|---|---|---|
| Complete a Pipeline step while typing | Yes, in supported Jenkinsfile documents | Search and retrieve step metadata; no inline completion |
| Inspect overloads, parameters, and return types | Hover | get_api_details / get_pipeline_step |
| Navigate to a source definition | Yes, when safe revision-pinned source evidence exists | Returns source provenance; the agent can explain it, but MCP does not open editor navigation |
| Show evidence-based diagnostics | Yes, for verified context/plugin/CPS/Sandbox evidence | Returns the same evidence in tool results; no live editor diagnostics |
| Search and paginate the catalog | Custom LSP requests | search_api and list_api |
| Retrieve catalog usage examples | Through symbol details/hover where available | get_usage_examples |
| Inspect controller/plugin inventory and package compatibility | Custom LSP requests | get_controller_inventory and get_package_compatibility |
| Ask an AI agent to use Jenkins-specific context | Not an agent interface | Yes; OpenCode is the worked example below |

Both interfaces read the same package and preserve its profile, source revisions, confidence, and unknown values. Neither proves that a Pipeline will execute successfully on a live controller.

## LSP: editor assistance for Jenkinsfiles

The LSP executable is jenkins-lsp. It communicates using LSP framing over stdin/stdout and receives the package selection in the LSP initialize options; it does not accept a --package command-line flag.

### Supported editor features

The server advertises completion, hover, definition, and document synchronization. Jenkins catalog intelligence is restricted to files named Jenkinsfile, Jenkinsfile.* or *.jenkinsfile. The VS Code extension selects Groovy documents at the client boundary, but the server intentionally returns no Jenkins catalog features for ordinary .groovy files.

**Completion**

- Scripted Pipeline contexts include registered Pipeline steps, catalogued globals, Shared Library symbols, and context-supported Jenkins Core/plugin types.
- Declarative Pipeline directives are offered outside steps; inside a script block, the completion scope switches to Scripted/Groovy symbols.
- A dot is a completion trigger. Context is derived from a bounded lexical scan; this is not a full Groovy parser.

**Hover**

Hover resolves exact-name overloads and shows available parameter and return-type details, plugin availability, source revision/provenance, execution constraints, confidence, and the selected package profile. Missing or uninspected facts remain unknown.

**Go to Definition**

A definition target is returned only when catalog evidence identifies a safe source path under a GitHub repository URL and a recorded revision. The URL is pinned to that revision. The LSP does not fetch the URL; symbols without safe source locations have no definition target.

**Diagnostics**

The server publishes diagnostics only when the relevant catalog evidence is explicitly verified:

- Declarative directives used outside their known valid contexts;
- symbols that depend on a plugin recorded as unavailable;
- symbols recorded as incompatible with Pipeline CPS;
- symbols recorded as rejected by Script Security Sandbox.

Unknown, missing, inferred, or not-inspected evidence does not become an error. Diagnostics include symbol/source identifiers, revision, provenance, confidence, and package profile.

### VS Code setup

Build the LSP executable and the repository's thin VS Code extension from the source checkout. The extension README contains the build and extension-host acceptance procedure.

~~~sh
cargo build --locked --release --manifest-path crates/jenkins-lsp/Cargo.toml
cd editors/vscode
npm ci
npm run compile
~~~

Configure the extension in the workspace settings:

~~~json
{
  "jenkinsKnowledge.serverPath": "/opt/jenkins-knowledge/bin/jenkins-lsp",
  "jenkinsKnowledge.packagePath": "/srv/jenkins-knowledge/production-pipelines",
  "jenkinsKnowledge.profile": "production-pipelines",
  "files.associations": {
    "Jenkinsfile": "groovy",
    "Jenkinsfile.*": "groovy",
    "*.jenkinsfile": "groovy"
  }
}
~~~

- serverPath points to the jenkins-lsp executable (or a command available on PATH).
- packagePath is an absolute path or a path relative to the workspace root. It must point to a built package directory containing manifest.json.
- profile is optional; when set, it must exactly match the profile recorded in the package manifest.
- The extension passes packagePath, profile, and workspaceRoot to the LSP during initialization. It does not start Builder.

Use **Jenkins Knowledge: Reload Knowledge Package** after changing the selected package. The extension sends jenkinsKnowledge/reload; a successful reload switches the Runtime, while a failed reload clears the previous package and its diagnostics instead of leaving stale symbols visible. Configuration changes are also handled through workspace/didChangeConfiguration.

See the [VS Code extension README](https://github.com/dmji/jenkins-knowledge/blob/master/editors/vscode/README.md) for the full smoke test and the [LSP implementation](https://github.com/dmji/jenkins-knowledge/blob/master/crates/jenkins-lsp/src/main.rs) for the protocol contract.

### LSP custom requests

In addition to standard editor requests, the server implements these read-only requests:

| Request | Purpose |
|---|---|
| jenkinsKnowledge/search | Search symbols with query, kind, source, controller, plugin, owner, type, context, sourceKind, confidence, and limit filters |
| jenkinsKnowledge/list | Paginated symbol listing with the same selectors plus offset |
| jenkinsKnowledge/get | Exact symbol details by nameOrId or id |
| jenkinsKnowledge/coverage | Catalog coverage metadata |
| jenkinsKnowledge/collection | Package and per-source collection outcomes |
| jenkinsKnowledge/controller | Selected controller metadata |
| jenkinsKnowledge/plugins | Plugin inventory and its availability status |
| jenkinsKnowledge/sources | Source identities and revisions |
| jenkinsKnowledge/reload | Explicitly select a local packagePath and optional profile |

Search/list limits are bounded to 1–100 results; list supports offsets up to 10,000. These requests inspect the selected local package. They do not run collection or contact Jenkins/Git.

## MCP: structured catalog access for agents

The executable is jenkins-mcp. It is a **local stdio server**, not an HTTP service. It opens exactly one Knowledge Package at process startup and serves newline-delimited JSON-RPC requests over stdin/stdout. Only protocol responses are written to stdout; startup errors go to stderr. It supports the MCP initialize handshake (protocol version 2025-06-18), tools, and resources. It does not implement prompts, resource subscriptions, or dynamic package reload.

Select the package explicitly with --package or -p. The environment variable JENKINS_KNOWLEDGE_PACKAGE is an alternative:

~~~sh
/opt/jenkins-knowledge/bin/jenkins-mcp \
  --package /srv/jenkins-knowledge/production-pipelines
~~~

If the package path changes, restart or reconnect the MCP server so it opens the new package. One process serves one package/profile.

### Implemented MCP tools

The current server exposes seven tools. Argument names and casing below are the actual JSON schemas; do not substitute camelCase for snake_case where the schema uses snake_case.

| Tool | Required arguments | Optional arguments / behavior |
|---|---|---|
| search_api | None | query, kind, source, controller, sourceKind, context, plugin, owner, type, confidence, offset, limit. Searches symbols and returns a bounded page. |
| list_api | None | kind, source, controller, sourceKind, context, plugin, owner, type, confidence, offset, limit. Lists symbols without a text query. |
| get_api_details | name_or_id | Exact symbol name or ID; returns details, source provenance, type references/members, and execution constraints. |
| get_usage_examples | name_or_id | Returns catalog-backed usage examples for the exact symbol. |
| get_pipeline_step | name | Returns Pipeline-step metadata, source, type relationships, and execution constraints. |
| get_controller_inventory | None | Returns controller identity, Core/plugin inventory, Java metadata availability, capabilities, collection profile, freshness, and provenance. Unknown inventory remains unknown. |
| get_package_compatibility | None | Optional targetEnvironment object with coreVersion, javaVersion, javaVendor, and plugins [{shortName, version}]. Compares recorded package metadata offline. |

search_api and list_api default to 50 results and accept limit 1–100 and offset 0–10,000. Responses include profile, offset, limit, total, returned, hasMore, nextOffset, and paginationLimitReached. The tool result contains both structuredContent and a text representation. Invalid tool arguments are returned as an MCP tool error (isError), not as a successful empty search.

A compatibility result of matched means the supplied fields match the recorded metadata; it is not proof that the target controller can execute every Pipeline or plugin API. If Java/plugin metadata was not collected, the report preserves that uncertainty.

### Catalog resources

resources/list advertises these read-only JSON resources; resources/read returns their contents:

| URI | Contents |
|---|---|
| jenkins://catalog/manifest | Package identity, schema/tool versions, provenance, checksums, and manifest metadata |
| jenkins://catalog/collection | Package-level completeness and per-source collection outcomes |
| jenkins://catalog/coverage | Catalog collection coverage |
| jenkins://catalog/controller | Selected controller metadata |
| jenkins://catalog/controller-inventory | Combined controller/Core/plugin inventory, capabilities, freshness, collection state, and provenance |
| jenkins://catalog/compatibility | Offline package-environment compatibility report |
| jenkins://catalog/plugins | Plugin inventory and status |
| jenkins://catalog/sources | Configured package sources and revisions |
| jenkins://catalog/types | Paginated Groovy types and resolved members |
| jenkins://catalog/libraries | Shared Library sources |
| jenkins://catalog/pipeline-steps | Paginated Pipeline steps and execution constraints |
| jenkins://catalog/globals | Paginated Pipeline/Shared Library globals |
| jenkins://catalog/declarative | Paginated Declarative Pipeline constructs |
| jenkins://catalog/library-symbols | Paginated Shared Library symbols |
| jenkins://catalog/plugin-symbols | Paginated plugin-provided symbols |
| jenkins://catalog/profiles/{profile} | Metadata for the one profile loaded by this process |

Paginated resource URIs accept offset and limit query parameters; the defaults are offset 0 and limit 50. Resource pagination is bounded to the same 1–100 limit and 0–10,000 offset range.

### Example tool calls

A client can search for a step, inspect its exact metadata, and then retrieve examples for a Shared Library symbol. The following are representative MCP tools/call requests; OpenCode normally selects and invokes tools on the agent's behalf.

~~~json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "search_api",
    "arguments": {
      "query": "archiveArtifacts",
      "kind": "step",
      "limit": 10
    }
  }
}
~~~

~~~json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "get_pipeline_step",
    "arguments": {
      "name": "archiveArtifacts"
    }
  }
}
~~~

~~~json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "get_usage_examples",
    "arguments": {
      "name_or_id": "deployApp"
    }
  }
}
~~~

These calls return data only if the selected package contains the requested symbol/example. An empty result means the package did not expose a matching entry; it does not prove that the live controller lacks the API.

## OpenCode: first-class AI-agent integration

OpenCode connects to local MCP servers over stdio. Its current configuration format places named servers under mcp.servers, uses type local, and passes the executable and its arguments as a command array. The server name prefixes tool names. See the official [OpenCode MCP server guide](https://opencode.ai/docs/mcp-servers/), [configuration reference](https://opencode.ai/docs/config/), and [CLI command reference](https://opencode.ai/docs/cli/commands).

### Prerequisites and package selection

1. Install OpenCode using its [official installation instructions](https://opencode.ai/docs/).
2. Download the Runtime archive for your operating system and CPU architecture from the [Jenkins Knowledge Releases](https://github.com/dmji/jenkins-knowledge/releases) page. Each archive contains both jenkins-runtime and jenkins-mcp under bin/.
3. Verify the archive's adjacent .sha256 file before extracting it. On Linux use sha256sum -c; on macOS use shasum -a 256 -c. On Windows use Get-FileHash as shown below.
4. Select a Knowledge Package directory containing manifest.json. For a quick demonstration, download the separate reference-package archive from the same release and verify its adjacent checksum. For useful controller-specific answers, an administrator must first build a package from the user's own Jenkins instance and configured Shared Libraries.

For Linux x86_64, the download and verification sequence is:

~~~sh
TAG=v0.0.1
PLATFORM=linux-x86_64
ARCHIVE=jenkins-runtime-$TAG-$PLATFORM.tar.gz
BASE=https://github.com/dmji/jenkins-knowledge/releases/download/$TAG
curl -fLO $BASE/$ARCHIVE
curl -fLO $BASE/$ARCHIVE.sha256
sha256sum -c $ARCHIVE.sha256
mkdir -p runtime && tar -xzf $ARCHIVE -C runtime
MCP=$PWD/runtime/bin/jenkins-mcp
~~~

Use PLATFORM=macos-aarch64 or macos-x86_64 on macOS and replace the checksum command with shasum -a 256 -c. The Windows archive contains jenkins-mcp.exe. In PowerShell:

~~~powershell
$tag = "v0.0.1"
$archive = "jenkins-runtime-$tag-windows-x86_64.tar.gz"
$base = "https://github.com/dmji/jenkins-knowledge/releases/download/$tag"
Invoke-WebRequest "$base/$archive" -OutFile $archive
Invoke-WebRequest "$base/$archive.sha256" -OutFile "$archive.sha256"
$expected = (Get-Content "$archive.sha256" -Raw).Trim().Split(' ')[0]
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "Runtime archive checksum mismatch" }
New-Item -ItemType Directory -Force runtime | Out-Null
tar -xzf $archive -C runtime
$MCP = (Resolve-Path "runtime/bin/jenkins-mcp.exe").Path
~~~

The project-provided reference package is a fixed Jenkins 2.356 / Java 17 demonstration snapshot. It is not a current inventory of your controller. If the matching release archive is not yet published, build the MCP executable from the source checkout:

~~~sh
cargo build --locked --release --manifest-path crates/jenkins-runtime/Cargo.toml --bin jenkins-mcp
~~~

Use target/release/jenkins-mcp as the command path on Linux/macOS, or target/release/jenkins-mcp.exe on Windows. To collect your own package, configure a Builder profile and run the Builder from the repository with JENKINS_USERNAME and JENKINS_API_TOKEN supplied to the process:

~~~sh
cargo run --locked --manifest-path crates/knowledge-builder/Cargo.toml -- \
  build profiles/your-controller.yml /srv/jenkins-knowledge/your-controller
~~~

The Builder is the administrative collection tool. It may contact only the configured Jenkins controller and Shared Library remotes. The Runtime/MCP process only reads the resulting package and requires no Jenkins/Git credentials.

### Configure OpenCode

Create or edit opencode.jsonc in the project where OpenCode will use Jenkins Knowledge. Use absolute paths; replace the executable and package paths with the actual locations on the machine:

~~~json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "jenkins_knowledge": {
        "type": "local",
        "command": [
          "/opt/jenkins-knowledge/bin/jenkins-mcp",
          "--package",
          "/srv/jenkins-knowledge/your-controller"
        ],
        "protocol": "legacy",
        "codemode": false
      }
    }
  }
}
~~~

The server name jenkins_knowledge is intentionally simple: with codemode disabled, OpenCode exposes tools using the server prefix, for example jenkins_knowledge_search_api and jenkins_knowledge_get_pipeline_step. The explicit legacy protocol setting uses the initialize handshake supported by this server; do not force the newer 2026-07-28 protocol. If you use the opencode mcp add shortcut instead of the JSONC example, Code Mode remains enabled by default; edit the generated server entry and set codemode to false to expose the direct tool names used here.

On Windows, use the extracted .exe path and Windows package path, for example C:/Tools/jenkins-knowledge/bin/jenkins-mcp.exe and C:/JenkinsKnowledge/production. Forward slashes are accepted by Windows path handling and avoid JSON backslash escaping.

The equivalent CLI command adds a local stdio server to the project configuration:

~~~sh
opencode mcp add jenkins_knowledge -- \
  /opt/jenkins-knowledge/bin/jenkins-mcp \
  --package /srv/jenkins-knowledge/your-controller
~~~

Add --global before the separator if the server should be available to all OpenCode projects for the current user. Do not configure this server as remote or give it an HTTP URL: the executable uses local stdio.

### Verify the connection and tools

1. From the project directory, run opencode mcp list. The jenkins_knowledge entry should report connected.
2. Start OpenCode and run /mcps. Confirm the server is connected and its tools are listed.
3. With codemode disabled, the tool names use the jenkins_knowledge_ prefix. Confirm search_api, get_api_details, list_api, get_usage_examples, get_pipeline_step, get_controller_inventory, and get_package_compatibility are available.
4. Ask the agent to list resources for jenkins_knowledge and read jenkins://catalog/manifest using OpenCode's built-in opencode_list_mcp_resources and opencode_read_mcp_resource tools.
5. Ask the agent to call search_api for a known Pipeline step from the selected package, then get_pipeline_step for the exact result. Confirm the response includes the package profile and source/confidence metadata.

If the server does not appear as connected, OpenCode could not complete the local process start or MCP initialize handshake. Follow the troubleshooting section below before asking the agent to use the tools.

### End-to-end Jenkins Pipeline scenario

Try this prompt in OpenCode after the server is connected:

> Use only the jenkins_knowledge MCP tools for Jenkins API facts. Find the Pipeline step named archiveArtifacts. Retrieve its full step details and any catalog-backed usage examples. Explain the parameters, required arguments, return/usage constraints, plugin dependencies, source revision, and confidence. Then draft a short Declarative Pipeline stage that uses only parameters supported by the returned evidence. Mark any unknown detail explicitly; do not invent arguments or claim that the snippet has been executed.

Expected behavior:

- The agent searches the selected package and retrieves the exact Pipeline-step details.
- It distinguishes a verified signature from inferred or unknown metadata and reports the source/profile.
- It uses get_usage_examples only when the catalog contains examples for the selected symbol.
- It produces an illustrative snippet grounded in the returned fields and clearly labels unknowns.

The agent cannot run the Pipeline, contact the Jenkins controller, verify credentials, or prove that a plugin is installed on a different target controller. Treat generated code as a proposal to review and test in Jenkins.

Other useful prompts:

- "Search for a Shared Library symbol named deployApp. Retrieve its details and usage examples. Show the recorded library revision and distinguish verified from inferred information."
- "Summarize the controller inventory and collection outcomes. Which Core/plugin/Java facts are known, unavailable, or not collected?"
- "Compare this package with Core 2.480.0, Java 17, and the plugin versions I provide. Report differences and explain why a match does not prove runtime compatibility."

### Troubleshooting OpenCode

| Symptom | Check |
|---|---|
| Server fails to spawn | Use an absolute executable path, confirm the archive matches the host OS/CPU, and on Unix ensure the binary has execute permission (chmod +x jenkins-mcp). |
| Server exits with missing package | Pass --package with the package directory, or set JENKINS_KNOWLEDGE_PACKAGE in the local server environment. The directory must contain manifest.json. |
| Package validation fails | Rebuild/re-download the complete package; do not edit catalog.json or manifest.json independently. Verify checksums and package schema. |
| Profile/data is unexpected | Check which package path OpenCode passes. The MCP process serves exactly one package and reports its profile in tool results. The reference package is not your live controller snapshot. |
| No tools are listed | Check opencode mcp list and /mcps, confirm type is local, command is an array, and the server name is under mcp.servers. Restart/reconnect after editing configuration. |
| Protocol initialization fails | Keep protocol set to legacy; this server implements protocol version 2025-06-18. Do not configure it as a remote HTTP endpoint or require protocol 2026-07-28. |
| New package contents are not visible | Reconnect/restart the local MCP process after changing the package. MCP does not provide a package reload tool. |

## Boundaries and limitations

- Builder is the only component that collects from Jenkins/Git. MCP and LSP read a local package; neither runs Pipeline code or changes Jenkins.
- A package is a snapshot. Its collection profile and per-source outcomes determine what the interfaces can know. Missing data remains unknown, not absent.
- MCP tools and resources expose catalog data; they are not a general Jenkins administration API, shell, Script Console, or build trigger.
- LSP diagnostics are evidence-based and conservative. They are not a Groovy compiler or complete Pipeline validator.
- OpenCode uses MCP only. This guide does not claim that OpenCode consumes LSP or that an MCP tool call triggers editor features.

## Implementation references

- [MCP server and tool/resource schemas](https://github.com/dmji/jenkins-knowledge/blob/master/crates/jenkins-runtime/src/bin/mcp.rs)
- [Runtime package API](https://github.com/dmji/jenkins-knowledge/blob/master/crates/jenkins-runtime/src/lib.rs)
- [LSP server](https://github.com/dmji/jenkins-knowledge/blob/master/crates/jenkins-lsp/src/main.rs)
- [VS Code client](https://github.com/dmji/jenkins-knowledge/tree/master/editors/vscode)
- [Builder usage and profile contract](https://github.com/dmji/jenkins-knowledge/blob/master/crates/knowledge-builder/README.md)
