FROM rust:1.96.0-bookworm AS builder

ARG TRUNK_VERSION=0.21.14
ARG WASM_PACK_VERSION=0.15.0

WORKDIR /workspace
RUN rustup target add wasm32-unknown-unknown \
    && cargo install trunk --version "${TRUNK_VERSION}" --locked \
    && cargo install wasm-pack --version "${WASM_PACK_VERSION}" --locked

COPY Cargo.toml Cargo.lock rust-toolchain.toml LICENSE ./
COPY apps ./apps
COPY crates ./crates

RUN wasm-pack build apps/pdf-worker \
        --target web \
        --release \
        --out-dir pkg \
        --out-name pdf_worker \
    && cd apps/frontend \
    && trunk build --release --locked

FROM nginx:1.29-alpine AS runtime

COPY infra/nginx/nginx.conf /etc/nginx/nginx.conf
COPY --from=builder /workspace/apps/frontend/dist /usr/share/nginx/html
COPY --from=builder /workspace/apps/pdf-worker/pkg /usr/share/nginx/html/assets/pdf-worker
COPY apps/pdf-worker/js/worker.js /usr/share/nginx/html/assets/pdf-worker/worker.js

RUN mkdir -p /tmp/client_temp /tmp/proxy_temp /tmp/fastcgi_temp \
        /tmp/uwsgi_temp /tmp/scgi_temp \
    && chown -R nginx:nginx /usr/share/nginx/html /tmp/client_temp /tmp/proxy_temp \
        /tmp/fastcgi_temp /tmp/uwsgi_temp /tmp/scgi_temp

USER nginx
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
    CMD ["wget", "--quiet", "--tries=1", "--spider", "http://127.0.0.1:8080/healthz"]

ENTRYPOINT ["nginx", "-g", "daemon off;"]
