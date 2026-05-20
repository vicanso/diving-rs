# Stage 1: build the React frontend.
FROM node:24-alpine AS webbuilder

COPY . /diving-rs
RUN apk update \
  && apk add git make \
  && cd /diving-rs \
  && make build-web

# Stage 2: build the Rust binary against glibc (Debian), so the resulting
# ELF runs on the distroless glibc runtime in stage 3.
FROM rust:slim-bookworm AS builder

COPY --from=webbuilder /diving-rs /diving-rs

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
       build-essential \
       pkg-config \
  && rm -rf /var/lib/apt/lists/*

RUN cd /diving-rs \
  && make release

# Pre-create a writable HOME for the non-root runtime user. The distroless
# runtime has no shell, so we cannot mkdir/chown inside it later.
RUN mkdir -p /home/rust/.diving \
  && chown -R 1000:1000 /home/rust

# Stage 3: distroless glibc runtime (no shell, no package manager).
# `panic = "abort"` in the release profile means we don't need libgcc_s,
# so `base-debian12` is sufficient — no need for the larger `cc-debian12`.
# Contents we rely on: glibc, ca-certificates, tzdata, /etc/passwd.
FROM gcr.io/distroless/base-debian12

EXPOSE 7001

COPY --from=builder /diving-rs/target/release/diving /usr/local/bin/diving
COPY --from=builder --chown=1000:1000 /home/rust /home/rust

ENV RUST_ENV=production

USER 1000:1000
WORKDIR /home/rust

# HEALTHCHECK removed: distroless has no `wget`/`curl` and no shell, so a
# Dockerfile-level probe is not possible. Use an orchestrator-level probe
# against `GET /ping` (Kubernetes livenessProbe, Docker Compose with an
# external curl container, etc.).
ENTRYPOINT ["/usr/local/bin/diving"]
CMD ["--mode", "web", "--listen", "0.0.0.0:7001"]
