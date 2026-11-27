# Troubleshooting

| Symptom | Likely cause | Action |
| --- | --- | --- |
| Core metadata failure | Controller unreachable, wrong context path, proxy/auth issue, insufficient read access | Check URL, credentials, and permissions; this mandatory source aborts collection. |
| Plugin inventory unknown/partial | Endpoint unavailable or permissions restricted | Inspect outcome; empty/unknown is not proof of no plugins. |
| GDSL unavailable/truncated | Endpoint, plugin, proxy, or response limits | Check context path and preserve partial outcome. |
| Shared Library scan failed | Git auth, invalid remote/ref, missing revision, unsupported/dynamic Groovy | Test Git access as Builder identity; pin full SHA; review scanner limits. |
| Script Console denied | Deep not approved or insufficient admin permission | Expected for Quick/Standard and read-only Deep negative control. |
| Package partial | Optional source failed, denied, truncated, unavailable, or skipped | Read manifest outcomes and catalog coverage; fix source then explicitly rebuild. |
| Runtime rejects package | Missing/mismatched files, schema/checksum error, unsafe path, symlink | Restore complete unmodified artifact. |
| Profile mismatch | Requested profile differs from manifest | Select matching package/profile. |
| MCP fails to start | Wrong executable/package path or invalid package | Check absolute paths; test same package with Runtime CLI. |
| VS Code has no completions | LSP/package path/profile wrong, file not treated as Jenkinsfile, symbol absent | Check extension output/settings, validate package, inspect coverage, reload package. |

When escalating, include tool version, sanitized profile, collection profile, package ID, Core/plugin versions, and source outcome/error code. Never attach tokens or raw sensitive controller responses.
