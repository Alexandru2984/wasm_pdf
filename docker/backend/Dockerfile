FROM rust:1.96.0-bookworm AS builder

WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml LICENSE ./
COPY apps ./apps
COPY crates ./crates
RUN cargo build --locked --release -p backend

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /usr/sbin/nologin app

COPY --from=builder /workspace/target/release/backend /usr/local/bin/pdf-editor-backend

USER app
EXPOSE 8080
ENV APP_HOST=0.0.0.0 \
    APP_PORT=8080 \
    RUST_LOG=backend=info

HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
    CMD ["curl", "--fail", "--silent", "http://127.0.0.1:8080/health/ready"]

ENTRYPOINT ["/usr/local/bin/pdf-editor-backend"]
