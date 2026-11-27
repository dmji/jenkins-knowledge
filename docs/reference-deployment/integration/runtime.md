# Configure Runtime CLI

Runtime is a local read-only consumer. It does not contact Jenkins/Git, use collection credentials, start Builder, or refresh package contents.

```sh
cargo build --locked --manifest-path crates/jenkins-runtime/Cargo.toml
cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package /srv/jenkins-knowledge/production-pipelines search deploy
cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package /srv/jenkins-knowledge/production-pipelines list --kind step
cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package /srv/jenkins-knowledge/production-pipelines get deployApp
cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package /srv/jenkins-knowledge/production-pipelines examples deployApp
cargo run --locked --manifest-path crates/jenkins-runtime/Cargo.toml -- --package /srv/jenkins-knowledge/production-pipelines step sh
```

Use the release executable in production rather than invoking Cargo per query. Package is mandatory unless `JENKINS_KNOWLEDGE_PACKAGE` is set. Optional `--profile` must match the manifest. Mount the package read-only. Results include profile, source snapshot, confidence, and provenance.
