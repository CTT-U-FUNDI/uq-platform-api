# ============================================================
# UQ Platform API — backend officiel (uq_platform_bin)
# Build : cargo --release ; Runtime : debian + python3 (KYC OCR/vision)
# ============================================================

# ---------- Build ----------
FROM rust:bookworm AS builder
WORKDIR /build

# Crates locaux (path dependencies ../uq_*_lib)
COPY uq_player_lib/ ./uq_player_lib/
COPY uq_black_key_standalone/ ./uq_black_key_standalone/
COPY uq_blockchain_lib/ ./uq_blockchain_lib/
COPY uq_cellular_lib/ ./uq_cellular_lib/
COPY uq_security_lib/ ./uq_security_lib/
COPY uq_core_lib/ ./uq_core_lib/
COPY uq_vault_lib/ ./uq_vault_lib/
COPY uq_chat_lib/ ./uq_chat_lib/
COPY uq_data_lib/ ./uq_data_lib/
COPY uq_platform_bin/ ./uq_platform_bin/

# Cache des deps : on build le manifeste d'abord si possible
WORKDIR /build/uq_platform_bin
RUN cargo build --release

# ---------- Runtime ----------
FROM debian:bookworm-slim

# Python + deps vision (KYC : OCR, face, liveness via scripts/kyc_python)
RUN apt-get update && apt-get install -y --no-install-recommends \
        python3 python3-pip ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && pip3 install --no-cache-dir --break-system-packages \
        opencv-python-headless numpy onnxruntime

WORKDIR /app/uq_platform_bin

COPY --from=builder /build/uq_platform_bin/target/release/uq_platform ./uq_platform
COPY uq_platform_bin/static ./static
COPY uq_platform_bin/scripts ./scripts
COPY uq_platform_bin/models ./models
RUN mkdir -p data users kyc_uploads snapshots wal 2>/dev/null || true

# Render injecte PORT ; le backend lit RUST_PORT (défaut 5000)
ENV RUST_HOST=0.0.0.0 \
    RUST_PORT=5000 \
    PYTHONUNBUFFERED=1
EXPOSE 5000

CMD ["sh", "-c", "RUST_PORT=${PORT:-5000} exec ./uq_platform"]
