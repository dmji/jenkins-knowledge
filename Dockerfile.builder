FROM debian:bookworm-slim AS builder-base
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git openssh-client \
    && rm -rf /var/lib/apt/lists/*

FROM builder-base AS builder
RUN groupadd --system --gid 10001 builder \
    && useradd --system --uid 10001 --gid builder --home-dir /home/builder --create-home builder
COPY --chown=builder:builder target/release/knowledge-builder /usr/local/bin/knowledge-builder
COPY --chown=builder:builder target/release/catalog-validator /usr/local/bin/catalog-validator
USER builder:builder
WORKDIR /workspace
ENTRYPOINT ["knowledge-builder"]
