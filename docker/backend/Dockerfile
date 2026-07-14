FROM rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS builder

WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml LICENSE ./
COPY apps ./apps
COPY crates ./crates
RUN cargo build --locked --release -p backend

FROM cgr.dev/chainguard/wolfi-base:latest@sha256:02dab76bd852a70556b5b2002195c8a5fdab77d323c433bf6642aab080489795 AS runtime

RUN apk add --no-cache ca-certificates openssl zstd

COPY --from=builder /workspace/target/release/backend /usr/local/bin/pdf-editor-backend

USER 65532:65532
EXPOSE 8080
ENV APP_HOST=0.0.0.0 \
    APP_PORT=8080 \
    RUST_LOG=backend=info

HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/pdf-editor-backend", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/pdf-editor-backend"]
