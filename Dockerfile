ARG BUILD_FROM=ghcr.io/hassio-addons/base:latest

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

# Build arguments
ARG BUILD_DATE
ARG BUILD_DESCRIPTION
ARG BUILD_NAME
ARG BUILD_REF
ARG BUILD_VERSION

# Labels
LABEL \
    io.hass.name="${BUILD_NAME}" \
    io.hass.description="${BUILD_DESCRIPTION}" \
    io.hass.type="addon" \
    io.hass.version="${BUILD_VERSION}" \
    org.opencontainers.image.title="${BUILD_NAME}" \
    org.opencontainers.image.description="${BUILD_DESCRIPTION}" \
    org.opencontainers.image.source="https://github.com/${BUILD_REPOSITORY}" \
    org.opencontainers.image.created="${BUILD_DATE}" \
    org.opencontainers.image.revision="${BUILD_REF}" \
    org.opencontainers.image.version="${BUILD_VERSION}"
