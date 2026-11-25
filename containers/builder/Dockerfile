FROM rust:1-bookworm AS build

WORKDIR /src

COPY crates/catalog-validator/ crates/catalog-validator/
COPY crates/declarative-model/ crates/declarative-model/
COPY crates/jenkins-gdsl-collector/ crates/jenkins-gdsl-collector/
COPY crates/shared-library-scanner/ crates/shared-library-scanner/
COPY crates/knowledge-builder/ crates/knowledge-builder/

RUN cargo build --release --manifest-path crates/knowledge-builder/Cargo.toml \
    && cargo build --release --manifest-path crates/catalog-validator/Cargo.toml

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates git openssh-client \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 builder \
    && useradd --uid 10001 --gid builder --home-dir /home/builder --create-home --shell /usr/sbin/nologin builder

COPY --from=build /src/crates/knowledge-builder/target/release/knowledge-builder /usr/local/bin/knowledge-builder
COPY --from=build /src/crates/catalog-validator/target/release/catalog-validator /usr/local/bin/catalog-validator
COPY schemas/ /opt/jenkins-knowledge/schemas/

ENV HOME=/tmp
ENV GIT_TERMINAL_PROMPT=0

WORKDIR /workspace
USER 10001:10001

ENTRYPOINT ["knowledge-builder"]
