# syntax=docker/dockerfile:1
# ---------------------------------------------------------------------------
# Bank Statement Fidelity Editor — container image for headless deployment.
#
# This image runs the *additive* `serve` subcommand (HTTP health surface +
# the existing worker runtime). It does NOT change the GUI/CLI architecture.
#
# It provisions the three Linux-native dependencies the app needs at runtime
# that a plain Rust builder would miss:
#   1. libpdfium.so   — pdfium-render binds to this at runtime.
#   2. Python 3 + PyMuPDF — the PyO3 bridge executes python/pymupdf_*.py.
#   3. fontconfig + freetype — egui/eframe + font handling link against these.
# ---------------------------------------------------------------------------

# ====== Stage 1: build =====================================================
# Pinned to match rust-toolchain.toml (channel = "1.88.0").
FROM rust:1.88-bookworm AS builder

# Build-time system deps:
#   - pkg-config + fontconfig/freetype headers for the GUI crates
#   - python3-dev provides libpython for the PyO3 link step (auto-initialize)
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libfontconfig1-dev \
        libfreetype6-dev \
        python3 \
        python3-dev \
        clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Leverage layer caching: copy manifests first, then sources.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo ./.cargo
COPY src ./src
COPY python ./python
COPY tests ./tests

# DUAL_CORE_PASSPHRASE is only needed to *run* the binary, not to compile it,
# but a couple of unit tests read it. The build below doesn't run tests.
RUN cargo build --release --bin dual-core-pdf-pipeline

# ====== Stage 2: runtime ===================================================
FROM debian:bookworm-slim AS runtime

# Runtime system deps (shared-object versions, not the -dev headers):
#   - libfontconfig1 / libfreetype6 : required by the linked binary
#   - python3 + libpython3.11       : PyO3 auto-initialize loads libpython
#   - python3-pip                   : to install PyMuPDF
#   - curl / ca-certificates        : fetch libpdfium + TLS roots for reqwest
RUN apt-get update && apt-get install -y --no-install-recommends \
        libfontconfig1 \
        libfreetype6 \
        python3 \
        python3-pip \
        libpython3.11 \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

# PyMuPDF + PyMuPDF Pro (imported by python/pymupdf_pro_integration.py via the
# PyO3 bridge). The integration module calls `pymupdf.pro.unlock(KEY)`, which
# lives in the separate `pymupdfpro` commercial package — plain `pymupdf` does
# NOT ship the `pymupdf.pro` module, so installing only `pymupdf` causes
# `ModuleNotFoundError: No module named 'pymupdf.pro'` at runtime.
#
# PyMuPDF Pro is published as a Linux x86_64 (glibc) wheel, which matches this
# Debian bookworm image. `pymupdfpro` depends on a matching `pymupdf`, so a
# single install line pulls both in compatible versions.
# --break-system-packages is required on Debian bookworm (PEP 668).
RUN pip3 install --no-cache-dir --break-system-packages pymupdfpro

# libpdfium.so — pdfium-render's Pdfium::default() binds to the system lib.
# Pulled from the bblanchon/pdfium-binaries "latest" release. NOTE: this is a
# third-party binary; pin to a specific release tag for reproducible builds.
RUN curl -fsSL \
        https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz \
        -o /tmp/pdfium.tgz \
    && mkdir -p /tmp/pdfium \
    && tar -xzf /tmp/pdfium.tgz -C /tmp/pdfium \
    && cp /tmp/pdfium/lib/libpdfium.so /usr/lib/libpdfium.so \
    && ldconfig \
    && rm -rf /tmp/pdfium /tmp/pdfium.tgz

WORKDIR /app

# Application binary + the runtime assets it reads relative to the cwd.
COPY --from=builder /build/target/release/dual-core-pdf-pipeline /usr/local/bin/dual-core-pdf-pipeline
COPY --from=builder /build/python ./python
COPY bank_templates ./bank_templates

# Writable working directories the app expects on startup.
RUN mkdir -p audit output logs cache/fonts

# The PyO3 bridge resolves python/ from cwd or this env var; set both clearly.
ENV PYO3_PYTHON_DIR=/app/python
ENV RUST_LOG=info
# PyMuPDF Pro's pro.unlock() scans system font directories by default, which
# is slow/noisy on a slim image. Point it at the app font cache and disable
# the broad auto-scan for deterministic, fast unlocks.
ENV PYMUPDFPRO_FONT_PATH=/app/cache/fonts
ENV PYMUPDFPRO_FONT_PATH_AUTO=0
# Railway injects $PORT; default for local `docker run` parity.
ENV PORT=8080

EXPOSE 8080

# Headless health-serving mode. DUAL_CORE_PASSPHRASE (≥16 chars) and any API
# keys must be supplied as deploy-time environment variables.
CMD ["dual-core-pdf-pipeline", "serve"]
