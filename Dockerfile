# Multi-stage Dockerfile for tsundoku
#
# Stages:
#   1. frontend-builder  -- builds the SPA from web/
#   2. chef               -- prepares Rust build environment + cargo-chef
#   3. planner            -- generates the dependency recipe
#   4. builder            -- builds dependencies (cached layer) then the app
#   5. runtime            -- minimal Alpine runtime image

# =============================================================================
# Stage 1: Build frontend
# =============================================================================
FROM node:22-alpine AS frontend-builder
WORKDIR /web

COPY web/package.json web/package-lock.json* ./
RUN npm install

COPY web/ ./
RUN npm run build

# =============================================================================
# Stage 2: Rust build environment
# =============================================================================
FROM rust:1.95-alpine AS chef

RUN apk add --no-cache \
    musl-dev \
    build-base \
    clang \
    mold \
    pkgconfig

RUN cargo install cargo-chef --locked
WORKDIR /app

# Use mold linker for faster linking
ENV RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=mold"

# =============================================================================
# Stage 3: Prepare recipe
# =============================================================================
FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY migration/ ./migration/
COPY crates/ ./crates/

RUN cargo chef prepare --recipe-path recipe.json

# =============================================================================
# Stage 4: Build dependencies (cached) + application
# =============================================================================
FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --features embed-frontend --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY migration/ ./migration/
COPY crates/ ./crates/
# td-config's STARTER_CONFIG_TOML include_str!()s this file at compile time
COPY config/tsundoku.example.toml ./config/tsundoku.example.toml

# Bring in the built frontend so rust-embed can pick it up at compile time
COPY --from=frontend-builder /web/dist ./web/dist

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --features embed-frontend && \
    cp /app/target/release/tsundoku /app/tsundoku

# =============================================================================
# Stage 5: Runtime
# =============================================================================
FROM alpine:latest AS runtime

RUN apk add --no-cache \
    ca-certificates \
    libgcc \
    libstdc++ \
    su-exec

RUN addgroup -g 1000 tsundoku && \
    adduser -D -u 1000 -G tsundoku tsundoku

WORKDIR /app

COPY --from=builder /app/tsundoku /usr/local/bin/tsundoku

RUN mkdir -p /app/data /app/config && \
    chown -R tsundoku:tsundoku /app && \
    chmod 777 /app/data /app/config

ENV PUID=1000
ENV PGID=1000

EXPOSE 8080

USER tsundoku:tsundoku

CMD ["tsundoku", "serve"]
