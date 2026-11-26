#!/usr/bin/env sh
set -eu

format_fixture=""
base_file=""
generated_lockfile=0
if [ ! -f Cargo.lock ]; then
  generated_lockfile=1
fi
cleanup() {
  if [ -n "$format_fixture" ]; then
    rm -f "$format_fixture"
  fi
  if [ -n "$base_file" ]; then
    rm -f "$base_file"
  fi
  if [ "$generated_lockfile" -eq 1 ]; then
    rm -f Cargo.lock
  fi
}
trap cleanup EXIT

expected_sha="${GITHUB_SHA:-$(git rev-parse HEAD)}"
actual_sha="$(git rev-parse HEAD)"
printf 'Validating commit %s\n' "$actual_sha"

# Check new Rust files and files whose base revision is already formatted.
# Preserve existing formatting debt instead of silently rewriting source.
base_sha="${VALIDATION_BASE_SHA:-$(git rev-parse HEAD^)}"
git diff --check "$base_sha" "$actual_sha"
if grep -E 'cargo fmt' .github/workflows/rust-ci.yml scripts/validate.sh | grep -v -- '--check'; then
  echo "validation scripts must never invoke rustfmt in write mode" >&2
  exit 1
fi
changed_rust="$(git diff --name-only "$base_sha" "$actual_sha" -- '*.rs')"
if [ -n "$changed_rust" ]; then
  printf '%s\n' "$changed_rust" | while IFS= read -r path; do
    if git cat-file -e "$base_sha:$path" 2>/dev/null; then
      base_file="$(mktemp "${TMPDIR:-/tmp}/rustfmt-base-XXXXXX.rs")"
      git show "$base_sha:$path" > "$base_file"
      if rustfmt --check --edition 2021 --config skip_children=true "$base_file" >/dev/null 2>&1; then
        rustfmt --check --edition 2021 --config skip_children=true "$path"
      else
        echo "Preserving pre-existing rustfmt drift in $path; new files remain checked."
      fi
      rm -f "$base_file"
      base_file=""
    else
      rustfmt --check --edition 2021 --config skip_children=true "$path"
    fi
  done
fi
format_fixture="$(mktemp "${TMPDIR:-/tmp}/rustfmt-regression-XXXXXX.rs")"
printf 'fn main( ){ println!("unformatted"); }\n' > "$format_fixture"
if rustfmt --check --edition 2021 "$format_fixture" >/dev/null 2>&1; then
  echo "rustfmt --check accepted an intentionally unformatted fixture" >&2
  exit 1
fi
rm -f "$format_fixture"
format_fixture=""
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p catalog-validator -- examples/catalog/catalog.json schemas/catalog.schema.json

rm -rf target/fixture-package target/fixture-package-repeat
SOURCE_DATE_EPOCH=0 cargo run -p knowledge-builder -- build profiles/fixture.yml target/fixture-package
SOURCE_DATE_EPOCH=0 cargo run -p knowledge-builder -- build profiles/fixture.yml target/fixture-package-repeat
diff -ru target/fixture-package target/fixture-package-repeat
cargo run -p catalog-validator -- target/fixture-package/catalog.json schemas/catalog.schema.json
cargo run -p jenkins-runtime -- --package target/fixture-package search deploy
cargo build -p jenkins-runtime --bin jenkins-mcp
cargo build -p jenkins-lsp
npm --prefix editors/vscode install --no-save --package-lock=false
npm --prefix editors/vscode run compile

cargo build --release -p knowledge-builder -p catalog-validator -p jenkins-runtime --bins
docker build --file Dockerfile.builder --tag jenkins-knowledge-builder:local .
docker build --file Dockerfile.runtime --tag jenkins-knowledge-runtime:local .
BUILDER_IMAGE=jenkins-knowledge-builder:local RUNTIME_IMAGE=jenkins-knowledge-runtime:local bash .github/scripts/container-smoke.sh

# Remove Cargo's transient lockfile only when the checkout did not provide one.
cleanup
trap - EXIT

# Validation must leave the checked-out commit untouched.
git diff --exit-code HEAD --
if [ -n "$(git status --porcelain --untracked-files=all)" ]; then
  echo "validation left untracked or modified files in the working tree:" >&2
  git status --short >&2
  exit 1
fi
