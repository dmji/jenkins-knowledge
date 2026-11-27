FROM ubuntu:24.04@sha256:562111b88caa79c71391c0c4d3c2114a7d5c35915c20561fd893cb1e07929e6b AS builder-base
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates git openssh-client \
    && rm -rf /var/lib/apt/lists/*

FROM builder-base AS builder
RUN groupadd --system --gid 10001 builder \
    && useradd --system --uid 10001 --gid builder --home-dir /home/builder --create-home builder \
    && mkdir -p /packages \
    && chown builder:builder /packages
COPY --chown=builder:builder target/release/knowledge-builder /usr/local/bin/knowledge-builder
COPY --chown=builder:builder target/release/catalog-validator /usr/local/bin/catalog-validator
USER builder:builder
WORKDIR /workspace
ENTRYPOINT ["knowledge-builder"]
