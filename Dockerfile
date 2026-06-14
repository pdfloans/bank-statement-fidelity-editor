# syntax=docker/dockerfile:1
# ---------------------------------------------------------------------------
# Bank Statement Fidelity Editor — container image for headless deployment.
#
# This image runs the *additive* `serve` subcommand (HTTP health surface +
# the existing worker runtime). It does NOT change the GUI/CLI architecture.
#
# All legacy Python, PyMuPDF, and pdfium dependencies have been removed,
# relying exclusively on the native Rust stack (oxidize-pdf, etc).
# ---------------------------------------------------------------------------

# ====== Stage 1: build =====================================================
# Pinned to match rust-toolchain.toml (channel = "1.88.0").
FROM rust:1.88-bookworm AS builder

# Build-time system deps:
#   - pkg-config + fontconfig/freetype headers for the GUI crates
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libfontconfig1-dev \
        libfreetype6-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Leverage layer caching: copy manifests first, then sources.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo ./.cargo
COPY src ./src
COPY tests ./tests

# DUAL_CORE_PASSPHRASE is only needed to *run* the binary, not to compile it,
# but a couple of unit tests read it. The build below doesn't run tests.
RUN cargo build --release --bin dual-core-pdf-pipeline

# ====== Stage 2: runtime ===================================================
FROM debian:bookworm-slim AS runtime

# Runtime system deps (shared-object versions, not the -dev headers):
#   - libfontconfig1 / libfreetype6 : required by the linked binary
#   - ca-certificates               : TLS roots for reqwest
RUN apt-get update && apt-get install -y --no-install-recommends \
        libfontconfig1 \
        libfreetype6 \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Application binary + the runtime assets it reads relative to the cwd.
COPY --from=builder /build/target/release/dual-core-pdf-pipeline /usr/local/bin/dual-core-pdf-pipeline
COPY bank_templates ./bank_templates

# Writable working directories the app expects on startup.
RUN mkdir -p audit output logs cache/fonts

ENV RUST_LOG=info
# Railway injects $PORT; default for local `docker run` parity.
ENV PORT=8080

EXPOSE 8080

# Headless health-serving mode. DUAL_CORE_PASSPHRASE (≥16 chars) and any API
# keys must be supplied as deploy-time environment variables.
CMD ["dual-core-pdf-pipeline", "serve"]
