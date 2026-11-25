FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p knowledge-builder

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/knowledge-builder /usr/local/bin/knowledge-builder
ENTRYPOINT ["knowledge-builder"]
