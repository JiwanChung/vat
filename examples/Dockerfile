# syntax=docker/dockerfile:1.7

# --- Stage 1: Build dependencies (cached layer) ---
FROM rust:1.77-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

# Install system dependencies for OpenSSL and protobuf
RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler \
        libprotobuf-dev \
        libssl-dev \
        pkg-config \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Build dependencies first (cached unless Cargo.lock changes)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build the application
COPY . .

ARG GIT_SHA="unknown"
ARG BUILD_TIME="unknown"
ENV GIT_SHA=${GIT_SHA}
ENV BUILD_TIME=${BUILD_TIME}

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin acme-api --bin acme-worker \
    && cp target/release/acme-api /usr/local/bin/ \
    && cp target/release/acme-worker /usr/local/bin/

# --- Stage 2: Migrations ---
FROM debian:bookworm-slim AS migrations
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://github.com/golang-migrate/migrate/releases/download/v4.17.0/migrate.linux-amd64.tar.gz \
    | tar -xz -C /usr/local/bin/
COPY migrations/ /migrations/

# --- Stage 3: Runtime (API) ---
FROM debian:bookworm-slim AS api

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 app \
    && useradd --uid 10001 --gid app --shell /bin/false --create-home app

COPY --from=builder /usr/local/bin/acme-api /usr/local/bin/
COPY --from=migrations /usr/local/bin/migrate /usr/local/bin/
COPY --from=migrations /migrations/ /migrations/
COPY deploy/config/ /etc/acme/

# Health check
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -f http://localhost:8080/healthz || exit 1

USER app
WORKDIR /home/app
EXPOSE 8080 9090 9102

# Metadata labels
LABEL org.opencontainers.image.source="https://github.com/acme/platform"
LABEL org.opencontainers.image.title="acme-api"
LABEL org.opencontainers.image.description="ACME Platform API Server"
LABEL org.opencontainers.image.revision="${GIT_SHA}"

ENTRYPOINT ["tini", "--"]
CMD ["acme-api", \
     "--config", "/etc/acme/production.toml", \
     "--bind", "0.0.0.0:8080", \
     "--metrics-bind", "0.0.0.0:9102"]

# --- Stage 4: Runtime (Worker) ---
FROM debian:bookworm-slim AS worker

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 app \
    && useradd --uid 10001 --gid app --shell /bin/false --create-home app

COPY --from=builder /usr/local/bin/acme-worker /usr/local/bin/
COPY deploy/config/ /etc/acme/

USER app
WORKDIR /home/app

LABEL org.opencontainers.image.title="acme-worker"
LABEL org.opencontainers.image.description="ACME Platform Background Worker"

ENTRYPOINT ["tini", "--"]
CMD ["acme-worker", \
     "--config", "/etc/acme/production.toml", \
     "--concurrency", "16", \
     "--queues", "orders,payments,notifications"]
