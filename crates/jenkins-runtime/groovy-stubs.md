# Groovy Stub Generator

Generate IDE-oriented Groovy declarations from a validated Knowledge Package. The catalog remains the source of truth; generated files are disposable outputs.

```sh
cargo test --manifest-path crates/jenkins-runtime/Cargo.toml --all-targets
cargo run --manifest-path crates/jenkins-runtime/Cargo.toml --bin groovy-stubs -- --package ./target/fixture-package --output ./target/fixture-package/jenkins-stubs.groovy
```

The generator validates the package through Runtime before writing output. It emits Pipeline step declarations and Shared Library globals, methods, properties, and types from the selected package only. Output includes the profile, package version, and catalog ID and is deterministic for unchanged package content.

Catalog signatures and overloads are retained in generated documentation and represented as method declarations. Parameter names are retained where available. Types outside the conservative Groovy/JDK type allowlist become `Object`; the generator does not guess types. Re-run the command after selecting a different profile's package to generate that profile's API surface.
