# Stage 1: build the React frontend.
FROM node:24-alpine AS webbuilder

COPY . /diving-rs
RUN apk update \
  && apk add git make \
  && cd /diving-rs \
  && make build-web

# Stage 2: build the Rust binary against glibc (Debian), so the resulting
# ELF runs on the debian-slim glibc runtime in stage 3.
FROM rust:slim-bookworm AS builder

COPY --from=webbuilder /diving-rs /diving-rs

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
       build-essential \
       pkg-config \
  && rm -rf /var/lib/apt/lists/*

RUN cd /diving-rs \
  && make release

# Pre-create a writable HOME for the non-root runtime user in the
# builder stage; stage 3 just COPYs it across with `--chown`.
RUN mkdir -p /home/rust/.diving \
  && chown -R 1000:1000 /home/rust

# Stage 3: debian-bookworm-slim glibc runtime.
#
# Familiar Debian environment — shell + apt available for `docker exec`
# debugging — with `libgcc_s.so.1` and `libstdc++` provided out of the
# box for our C dependencies (zstd-sys, blake3, mimalloc).
FROM debian:bookworm-slim

EXPOSE 7001

# ca-certificates: outbound TLS to Docker registries (`https://...`).
# tzdata: accurate timestamps in reports. `--no-install-recommends` +
# cache cleanup keeps the runtime layer compact.
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
       ca-certificates \
       tzdata \
  && apt-get clean \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /diving-rs/target/release/diving /usr/local/bin/diving
COPY --from=builder --chown=1000:1000 /home/rust /home/rust

ENV RUST_ENV=production

USER 1000:1000
WORKDIR /home/rust

# HEALTHCHECK omitted: image ships no `wget`/`curl` by default. Use an
# orchestrator-level probe against `GET /ping` (Kubernetes
# livenessProbe, Docker Compose with an external `curl` sidecar, etc.),
# or `apt-get install -y wget` above and re-enable HEALTHCHECK here.
ENTRYPOINT ["/usr/local/bin/diving"]
CMD ["--mode", "web", "--listen", "0.0.0.0:7001"]
