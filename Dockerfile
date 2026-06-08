# syntax=docker/dockerfile:1
#
# Multi-stage image for the smooth-operator WebSocket server.
#
# ──────────────────────────────────────────────────────────────────────────
#  CROSS-REPO BUILD CONTEXT (read this before `docker build`)
# ──────────────────────────────────────────────────────────────────────────
# The Rust workspace (`rust/Cargo.toml`) has a *path* dependency on a SIBLING
# repository that lives OUTSIDE this repo:
#
#     smooai-smooth-operator-core = { path = "../../smooth-operator-core/rust/smooth-operator-core" }
#
# Relative to the workspace at `rust/`, that resolves to
# `<repo-parent>/smooth-operator-core/rust/smooth-operator-core`. A Docker build context
# rooted at this repo alone therefore CANNOT see it and the build will fail at
# the `cargo build` step with an unresolved path dependency.
#
# Until `smooai-smooth-operator-core` is published to crates.io (roadmap Phase 0,
# which removes the path dep), the image MUST be built with a context that spans
# BOTH repos. Lay them out as siblings (the standard `~/dev/smooai/` layout):
#
#     <parent>/
#       ├── smooth-operator/        (this repo)
#       └── smooth-operator-core/   (the engine; sibling)
#
# then build from the PARENT directory, pointing -f at this Dockerfile:
#
#     docker build \
#       -f smooth-operator/Dockerfile \
#       -t smooth-operator:dev \
#       <parent>
#
# i.e. from `~/dev/smooai`:
#
#     docker build -f smooth-operator/Dockerfile -t smooth-operator:dev .
#
# Inside the build the two repos appear at `/src/smooth-operator` and
# `/src/smooth-operator-core`, preserving the `../../smooth-operator-core/...` relative
# path the workspace expects. See deploy/k8s/README.md for the full story.
#
# Once `smooai-smooth-operator-core` is on crates.io and `rust/Cargo.toml` switches
# the workspace dep to a version, this Dockerfile can be simplified to a
# single-repo context (`docker build -t … smooth-operator`) by dropping
# the sibling COPY and the `WORKDIR /src/smooth-operator/rust` can build
# directly.
# ──────────────────────────────────────────────────────────────────────────

# ── Builder ────────────────────────────────────────────────────────────────
# Pin a Debian-bookworm Rust toolchain. The workspace is edition 2021; any
# recent stable (1.74+) satisfies axum 0.8 / tokio 1. `rust:1-bookworm` tracks
# the latest stable 1.x on bookworm so the runtime glibc matches the
# `debian:bookworm-slim` final stage.
FROM rust:1-bookworm AS builder

# Build deps for the postgres adapter / TLS-capable crates (openssl, pkg-config).
# Kept minimal; the server bin itself is pure-Rust + axum but the workspace
# pulls the postgres adapter into the build graph.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Copy BOTH repos (the context spans the parent dir — see header). Order:
# sibling engine first, then this repo, so the `../../smooth-operator-core` path dep
# resolves from `/src/smooth-operator/rust`.
COPY smooth-operator-core/ /src/smooth-operator-core/
COPY smooth-operator/ /src/smooth-operator/

WORKDIR /src/smooth-operator/rust

# Build only the server binary in release mode.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --locked -p smooai-smooth-operator-server \
    && cp target/release/smooth-operator-server /smooth-operator-server

# ── Runtime ────────────────────────────────────────────────────────────────
# Slim Debian with ca-certificates so the server can reach the HTTPS LLM
# gateway (https://llm.smoo.ai/v1) and a TLS Postgres.
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Non-root runtime user.
RUN groupadd --system --gid 10001 smooth \
    && useradd --system --uid 10001 --gid smooth --no-create-home --shell /usr/sbin/nologin smooth

COPY --from=builder /smooth-operator-server /usr/local/bin/smooth-operator-server

USER 10001:10001

# Default WS port (overridable via SMOOTH_AGENT_PORT). Documented in
# rust/.../config.rs. NOTE: the server currently binds 127.0.0.1 — for k8s it
# must bind 0.0.0.0. See deploy/k8s/README.md "0.0.0.0 bind follow-up".
ENV SMOOTH_AGENT_PORT=8787
EXPOSE 8787

ENTRYPOINT ["/usr/local/bin/smooth-operator-server"]
