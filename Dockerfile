FROM rust:1.87 AS builder
WORKDIR /app

COPY gui ./gui
COPY rust ./rust

RUN cargo build --manifest-path rust/Cargo.toml --release -p server

FROM debian:bookworm-slim
WORKDIR /app

COPY --from=builder /app/rust/target/release/server /usr/local/bin/cloud-code

ENV CLOUD_CODE_HOST=0.0.0.0
ENV CLOUD_CODE_PORT=8787
ENV CLOUD_CODE_WORKSPACE_ROOT=/app

EXPOSE 8787

CMD ["cloud-code"]
