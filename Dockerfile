ARG BUILD_FROM

# --- Build stage: compile Rust binary on Alpine (musl) ---
FROM rust:1-alpine AS builder

RUN apk add --no-cache build-base

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release

# --- Runtime stage: HA base image with bashio ---
FROM ${BUILD_FROM}

COPY --from=builder /build/target/release/grillsense /usr/bin/grillsense
COPY run.sh /
RUN chmod a+x /run.sh

CMD ["/run.sh"]
