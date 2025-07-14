# Use the official Rust image as a base.
FROM rust:latest AS builder

# Set the working directory.
WORKDIR /usr/src/app

# Copy manifests to leverage Docker cache layers.
COPY Cargo.toml Cargo.lock ./

# Create a dummy project to cache dependencies.
RUN mkdir src && \
    echo "fn main(){}" > src/main.rs && \
    cargo build --release

# Copy the actual source code.
COPY src ./src
COPY static_root ./static_root

# Build the final release binary.
RUN cargo build --release

# --- Final Stage ---
# Use a minimal base image for a smaller final container.
FROM debian:bookworm-slim

# Copy the compiled binary from the builder stage.
COPY --from=builder /usr/src/app/target/release/websrv /usr/local/bin/websrv
# Copy the static assets.
COPY --from=builder /usr/src/app/static_root ./static_root

# Expose the port the server runs on.
EXPOSE 8080

# Set the command to run the server.
CMD ["/usr/local/bin/websrv"]