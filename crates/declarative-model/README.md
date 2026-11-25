# Declarative Pipeline model

This crate exposes a conservative, version-neutral model of core Declarative Pipeline constructs as canonical catalog symbols. It does not parse Jenkinsfiles or execute Pipeline code.

## Use

```rust
use declarative_model::{catalog_fragment, validate_fragment};

let fragment = catalog_fragment("jenkins-controller-2.479.1");
let schema: serde_json::Value =
    serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))?;
validate_fragment(&fragment, &schema, "jenkins-controller-2.479.1")?;
```

catalog_symbols emits only declarative-directive symbols. Each symbol carries its valid parent contexts and conservative nesting/availability constraints. All entries use inferred confidence because installed plugin and Jenkins-version differences cannot be established by this static model. A source collector may replace that confidence only when it has authoritative evidence.

The model includes pipeline, agent, stages, stage, steps, post, when, environment, tools, options, parameters, triggers, input, libraries, parallel, matrix, and script. It intentionally does not invent plugin-specific condition names, option names, or parameter signatures.

## Checks

```sh
cargo fmt --manifest-path crates/declarative-model/Cargo.toml -- --check
cargo test --manifest-path crates/declarative-model/Cargo.toml --all-targets
cargo clippy --manifest-path crates/declarative-model/Cargo.toml --all-targets -- -D warnings
```
