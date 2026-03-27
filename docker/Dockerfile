# syntax=docker/dockerfile:1

FROM node:22-alpine AS ui-builder
WORKDIR /app/ui
COPY ui/package.json ui/pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile
COPY ui/ .
RUN pnpm build

FROM lukemathwalker/cargo-chef:0.1.77-rust-1.94-alpine3.21 AS chef
RUN apk add --no-cache musl-dev
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
RUN cargo chef prepare --recipe-path recipe.json --bin rustmail

FROM chef AS rust-builder
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef cook --release --no-default-features --recipe-path recipe.json
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY --from=ui-builder /app/ui/dist ui/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --no-default-features --bin rustmail

FROM alpine:3.21
RUN addgroup -S rustmail && adduser -S -G rustmail rustmail
COPY --from=rust-builder /app/target/release/rustmail /usr/local/bin/rustmail
RUN mkdir -p /data && chown rustmail:rustmail /data
USER rustmail
EXPOSE 1025 8025
VOLUME ["/data"]
ENV RUSTMAIL_DB_PATH=/data/rustmail.db \
    RUSTMAIL_BIND=0.0.0.0
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -qO /dev/null http://127.0.0.1:8025/api/v1/messages || exit 1
ENTRYPOINT ["rustmail"]
