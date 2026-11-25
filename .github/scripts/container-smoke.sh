#!/usr/bin/env bash
set -euo pipefail

builder_image="${BUILDER_IMAGE:-jenkins-knowledge-builder:ci}"
runtime_image="${RUNTIME_IMAGE:-jenkins-knowledge-runtime:ci}"
workspace="$(pwd)"
package_parent="$(mktemp -d)"
trap 'rm -rf "$package_parent"' EXIT
chmod 0777 "$package_parent"

docker run --rm \
  --volume "$workspace:/workspace:ro" \
  --volume "$package_parent:/output" \
  --workdir /workspace \
  "$builder_image" build profiles/fixture.yml /output/package

docker run --rm \
  --volume "$package_parent/package:/package:ro" \
  "$runtime_image" --package /package search deploy

docker run --rm --entrypoint groovy-stubs \
  --volume "$package_parent/package:/package:ro" \
  --volume "$package_parent:/output" \
  "$runtime_image" --package /package --output /output/stubs.groovy
test -s "$package_parent/stubs.groovy"

docker run --rm --entrypoint sh "$runtime_image" -ec '
  test "$(id -u)" -ne 0
  for tool in git cargo rustc knowledge-builder catalog-validator; do
    if command -v "$tool" >/dev/null 2>&1; then
      echo "unexpected Builder tool in Runtime image: $tool" >&2
      exit 1
    fi
  done
  for tool in jenkins-runtime jenkins-mcp groovy-stubs; do
    command -v "$tool" >/dev/null
  done
  test ! -e /root/.ssh
  test ! -e /root/.cargo
'
