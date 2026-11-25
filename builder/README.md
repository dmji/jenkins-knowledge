# Builder

The Rust Builder assembles a versioned Knowledge Package from an explicit profile.

Run the offline fixture package:

```sh
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- build profiles/fixture.yml ./target/fixture-package
```

For profile format, live Jenkins/Git collection, deterministic output, package layout, and failure behavior, see [knowledge-builder](../crates/knowledge-builder/README.md).

Credentials are injected at build time through environment variables or a secret provider. Never serialize credentials into profiles, catalogs, logs, or package metadata.
