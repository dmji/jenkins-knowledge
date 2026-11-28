#!/usr/bin/env python3
"""Smoke-test the packaged Jenkins Knowledge LSP binary against a package."""

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path


def frame(message):
    body = json.dumps(message, separators=(",", ":")).encode()
    return b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n" + body


def parse_frames(data):
    messages = []
    while data:
        separator = data.find(b"\r\n\r\n")
        if separator < 0:
            raise AssertionError(f"incomplete LSP header: {data[:200]!r}")
        header = data[:separator]
        match = re.search(rb"(?im)^Content-Length:\s*(\d+)\s*$", header)
        if not match:
            raise AssertionError(f"missing Content-Length header: {header!r}")
        length = int(match.group(1))
        start = separator + 4
        end = start + length
        if len(data) < end:
            raise AssertionError("incomplete LSP message body")
        messages.append(json.loads(data[start:end]))
        data = data[end:]
    return messages


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: smoke-lsp.py <jenkins-lsp-binary> <knowledge-package-dir>")

    binary = Path(sys.argv[1]).resolve()
    package = Path(sys.argv[2]).resolve()
    manifest_path = package / "manifest.json"
    if not binary.is_file() or not manifest_path.is_file():
        raise AssertionError("LSP executable or package manifest is missing")

    profile = json.loads(manifest_path.read_text(encoding="utf-8"))["profile"]
    with tempfile.TemporaryDirectory(prefix="jenkins-knowledge-lsp-") as temp:
        workspace = Path(temp).resolve()
        initialize = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": None,
                "rootUri": None,
                "rootPath": str(workspace),
                "capabilities": {},
                "clientInfo": {"name": "jenkins-knowledge-release-smoke", "version": "1"},
                "initializationOptions": {
                    "packagePath": str(package),
                    "profile": profile,
                    "workspaceRoot": str(workspace),
                },
            },
        }
        messages = b"".join(
            [
                frame(initialize),
                frame({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
                frame(
                    {
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "jenkinsKnowledge/coverage",
                        "params": {},
                    }
                ),
                frame({"jsonrpc": "2.0", "id": 3, "method": "shutdown", "params": {}}),
                frame({"jsonrpc": "2.0", "method": "exit"}),
            ]
        )

        process = subprocess.Popen(
            [str(binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            stdout, stderr = process.communicate(input=messages, timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate()
            raise AssertionError(f"LSP process did not shut down; stderr={stderr.decode(errors='replace')}")

    if process.returncode != 0:
        raise AssertionError(
            f"LSP exited with {process.returncode}; stderr={stderr.decode(errors='replace')}"
        )

    responses = {message.get("id"): message for message in parse_frames(stdout) if "id" in message}
    initialized = responses.get(1, {}).get("result")
    if not initialized:
        raise AssertionError(f"LSP initialize response missing: {responses.get(1)}")
    capabilities = initialized.get("capabilities", {})
    assert capabilities.get("completionProvider"), capabilities
    assert capabilities.get("hoverProvider") is True, capabilities
    assert capabilities.get("definitionProvider") is True, capabilities

    coverage = responses.get(2, {}).get("result")
    if not coverage or coverage.get("profile") != profile:
        raise AssertionError(f"LSP did not open the selected package: {coverage!r}")
    if 3 not in responses or "error" in responses[3]:
        raise AssertionError(f"LSP shutdown response missing or failed: {responses.get(3)}")

    print(f"LSP smoke passed: profile={profile}, completion/hover/definition advertised")


if __name__ == "__main__":
    main()
