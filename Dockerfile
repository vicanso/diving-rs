# Stage 1: build the React frontend.
FROM node:24-alpine AS webbuilder

COPY . /diving-rs
RUN apk update \
  && apk add git make \
  && cd /diving-rs \
  && make build-web

# Stage 2: build the Rust binary against glibc (Debian). Pinned Rust
# version for reproducible builds; the non-slim base already ships
# build-essential + pkg-config, so no extra apt install is needed for
# our C dependencies (zstd-sys, blake3, mimalloc).
FROM rust:1.95.0 AS builder

COPY --from=webbuilder /diving-rs /diving-rs

RUN cd /diving-rs \
  && make release

# Stage 3: debian-trixie-slim glibc runtime (Debian 13).
#
# Familiar Debian environment — shell + apt available for `docker exec`
# debugging — with `libgcc_s.so.1` and `libstdc++` provided out of the
# box for our C dependencies (zstd-sys, blake3, mimalloc).
FROM debian:trixie-slim

EXPOSE 7001

# reqwest + rustls verifies registry TLS against the system trust store
# (rustls-platform-verifier → openssl-probe's default
# /etc/ssl/certs/ca-certificates.crt), and trixie-slim ships no CA bundle.
# Instead of `apt-get install ca-certificates` here — which permanently bakes
# ~1.6 MiB of dpkg/debconf cruft into the layer (the rewritten
# /var/lib/dpkg/status DB survives `rm -rf /var/lib/apt/lists/*`) — copy the
# bundle the rust:1.95.0 builder already carries. No package manager runs in
# this stage, so there is no apt/dpkg waste to clean up.
#
# tzdata is intentionally not installed: reports and access logs render in UTC
# (the tracing timer is pinned to UTC in main.rs; analysis timestamps are
# chrono::Utc). The terminal UI's local-time formatting degrades to UTC when
# /etc/localtime is absent, and is unused in `--mode web` anyway.
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

# Service-style user pinned to UID 1000 for backward compat with the
# README's `chown -R 1000:1000 ./diving` step. `-m` creates /home/rust so
# `home::home_dir()`'s passwd-based fallback resolves correctly;
# `/bin/false` blocks interactive login at the shell level (an explicit
# `docker exec -it … bash` still works for debugging). `~/.diving` is
# pre-created and owned by the user so the first run never has to mkdir.
RUN useradd -u 1000 -m -s /bin/false rust \
  && mkdir -p /home/rust/.diving \
  && chown -R rust:rust /home/rust

COPY --from=builder --chown=rust:rust --chmod=755 \
     /diving-rs/target/release/diving /usr/local/bin/diving

# Docker does not auto-set $HOME on USER switch — it inherits `/root`
# from the parent image. Set it explicitly so UID 1000 can write
# `~/.diving` (else `home::home_dir()` returns /root and panics with
# EACCES on directory creation).
ENV RUST_ENV=production \
    HOME=/home/rust

USER rust
WORKDIR /home/rust

# HEALTHCHECK omitted: image ships no `wget`/`curl` by default. Use an
# orchestrator-level probe against `GET /ping` (Kubernetes
# livenessProbe, Docker Compose with an external `curl` sidecar, etc.),
# or `apt-get install -y wget` above and re-enable HEALTHCHECK here.
ENTRYPOINT ["/usr/local/bin/diving"]
CMD ["--mode", "web", "--listen", "0.0.0.0:7001"]
