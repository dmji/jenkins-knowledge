# MCP server and OpenCode

The **jenkins-mcp** executable exposes one validated Knowledge Package over local MCP stdio. It is not an HTTP endpoint: the client starts the process and exchanges newline-delimited JSON-RPC over stdin/stdout. One process serves one package/profile and reads it once at startup.

In production, use the **jenkins-mcp** executable from the platform tool bundle. It is located under bin/jenkins-mcp in the extracted archive. Select the package with **--package** or **JENKINS_KNOWLEDGE_PACKAGE**. The process does not contact Jenkins/Git, execute Pipeline steps, refresh a package, or mutate package contents.

## OpenCode

OpenCode is the first fully documented AI-agent integration. Its current configuration format uses **mcp.servers**, **type: local**, and a command array. Do not use the older **mcpServers** shape or configure this stdio process as a remote HTTP server.

~~~json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "jenkins_knowledge": {
        "type": "local",
        "command": [
          "/opt/jenkins-knowledge/tools/bin/jenkins-mcp",
          "--package",
          "/srv/jenkins-knowledge/production-pipelines"
        ],
        "protocol": "legacy",
        "codemode": false
      }
    }
  }
}
~~~

Use absolute paths and replace them with the executable and package on the OpenCode host. The server implements MCP protocol version 2025-06-18 and supports the legacy initialize handshake. The explicit protocol setting avoids requesting a newer protocol revision that this server does not implement.

Verify the connection with:

~~~sh
opencode mcp list
~~~

In the OpenCode TUI, use **/mcps** to inspect the connected server and its tools. With Code Mode disabled, tools are exposed with the **jenkins_knowledge_** prefix. The complete setup, expected tool list, realistic prompts, package selection, and troubleshooting are in the [LSP and MCP interface guide]({{ '/interfaces.html' | relative_url }}).

## MCP capabilities

The current server implements seven read-only tools: search_api, list_api, get_api_details, get_usage_examples, get_pipeline_step, get_controller_inventory, and get_package_compatibility. It also exposes package manifest, collection outcomes, coverage, controller inventory, compatibility, plugin/source inventory, types, Pipeline steps, globals, Declarative constructs, Shared Library/plugin symbols, and selected-profile resources.

Search/list and paginated resources default to 50 results and accept limits from 1 to 100. Pagination offsets are bounded to 10,000. Tool and resource results preserve package profile, source provenance, confidence, and unknown values. Compatibility comparison is offline; a metadata match does not prove that the target controller can execute every API.

The server exposes tools and resources only. It does not expose MCP prompts, resource subscriptions, or a dynamic package-reload tool. Reconnect/restart the process after selecting a different package.

See the [full LSP/MCP feature guide]({{ '/interfaces.html' | relative_url }}), [Runtime source](https://github.com/dmji/jenkins-knowledge/tree/master/crates/jenkins-runtime), and the [official OpenCode MCP documentation](https://opencode.ai/docs/mcp-servers/).
