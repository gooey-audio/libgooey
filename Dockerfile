# Multi-stage Dockerfile for building gooey audio engine
# Stage 1: WASM builder

FROM rust:1.87 AS wasm-builder

# Install wasm-pack
RUN curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

WORKDIR /app

# Copy source files
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY build.rs cbindgen.toml ./

# Build WASM package with web feature
RUN wasm-pack build --target web --release --no-default-features --features web --out-dir /wasm-output

# Final output is in /wasm-output
