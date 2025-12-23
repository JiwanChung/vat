# Build stage
FROM rust:1.75-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache musl-dev openssl-dev pkgconfig

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy main to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy source code
COPY src ./src

# Build the application
ARG BUILD_VERSION=dev
ENV BUILD_VERSION=${BUILD_VERSION}
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM alpine:3.19 AS runtime

LABEL maintainer="dev@example.com"
LABEL version="1.0.0"
LABEL description="Production image for vat"

# Install runtime dependencies
RUN apk add --no-cache ca-certificates libgcc

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/vat /usr/local/bin/

# Create non-root user
RUN addgroup -S appgroup && adduser -S appuser -G appgroup
USER appuser

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD vat --version || exit 1

EXPOSE 8080

ENTRYPOINT ["vat"]
CMD ["--help"]
