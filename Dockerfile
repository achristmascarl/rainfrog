FROM lukemathwalker/cargo-chef:latest-rust-bookworm AS chef
WORKDIR /app

# Cache dependencies
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Build application
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release

# Create runtime image
FROM debian:bookworm-slim AS runtime
WORKDIR /usr/src/app

# Create non-root user
RUN useradd -m -s /bin/bash rainfrog

# Copy the binary from the builder image
COPY --from=builder /app/target/release/rainfrog /usr/local/bin/rainfrog
COPY --from=builder /app/target/release/deps /usr/lib

USER rainfrog

HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
  CMD pidof rainfrog || exit 1

# Command to construct the full connection options using environment variables
CMD ["bash", "-c", "rainfrog --username=$username --password=$password --host=$hostname --port=$db_port --database=$db_name --driver=$db_driver"]
