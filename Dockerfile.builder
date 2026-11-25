FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
COPY target/release/knowledge-builder /usr/local/bin/knowledge-builder
ENTRYPOINT ["knowledge-builder"]
