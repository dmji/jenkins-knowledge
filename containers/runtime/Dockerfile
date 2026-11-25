FROM rust:1-bookworm AS build

WORKDIR /src

COPY crates/catalog-validator/ crates/catalog-validator/
COPY crates/jenkins-runtime/ crates/jenkins-runtime/

RUN cargo build --release --manifest-path crates/jenkins-runtime/Cargo.toml --bins

FROM debian:bookworm-slim

RUN groupadd --gid 10001 jenkins \
    && useradd --uid 10001 --gid jenkins --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin jenkins

COPY --from=build /src/crates/jenkins-runtime/target/release/jenkins-runtime /usr/local/bin/jenkins-runtime
COPY --from=build /src/crates/jenkins-runtime/target/release/jenkins-mcp /usr/local/bin/jenkins-mcp
COPY --from=build /src/crates/jenkins-runtime/target/release/groovy-stubs /usr/local/bin/groovy-stubs
COPY schemas/ /opt/jenkins-knowledge/schemas/

ENV JENKINS_KNOWLEDGE_PACKAGE=/package

WORKDIR /opt/jenkins-knowledge
USER 10001:10001

ENTRYPOINT ["jenkins-runtime"]
