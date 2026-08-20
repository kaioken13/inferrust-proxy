# --- Stage 1: Build Stage ---
FROM rust:latest AS builder

WORKDIR /app

# Copy Cargo manifests for dependency caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy project to build and cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy real source code and tokenizer configuration
COPY src ./src
COPY tokenizer.json ./

# Compile the actual project in release mode
RUN touch src/main.rs && cargo build --release

# --- Stage 2: Runtime Stage ---
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies (ca-certificates for HTTPS, SSL libs)
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy compiled binary from builder stage
COPY --from=builder /app/target/release/inferrust-proxy /app/inferrust-proxy

# Copy tokenizer file required for prompt token counting
COPY tokenizer.json /app/tokenizer.json

# Expose proxy port
EXPOSE 3000

# Run the proxy
CMD ["./inferrust-proxy"]