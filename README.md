# diving-rs

[![Release](https://img.shields.io/github/v/release/vicanso/diving-rs)](https://github.com/vicanso/diving-rs/releases)
[![Docker Pulls](https://img.shields.io/docker/pulls/vicanso/diving)](https://hub.docker.com/r/vicanso/diving)
[![License](https://img.shields.io/github/license/vicanso/diving-rs)](./LICENSE)

[中文](./README-zh.md)

**Dive into every layer of a Docker image — find wasted space, leaked secrets, and bloat in seconds.**

A single, fast Rust binary that pulls images straight from any registry and shows you exactly what's inside. **No Docker daemon, no root, no dependencies.** Works on Linux, macOS and Windows.

![](./assets/diving-terminal.gif)

## Why diving?

- ⚡ **Fast & standalone** — one static binary. Pulls layers directly from Docker Hub / any V2 registry, a local docker client, or a `.tar` file. Layers are cached and downloads resume automatically.
- 🔍 **Layer-by-layer explorer** — an interactive TUI to walk the filesystem of every layer, with added / modified / removed files colorized.
- 📉 **Waste & bloat detection** — efficiency score, wasted bytes, cross-layer duplicate files, oversized layers, package-manager caches, dev/build artifacts, and a reconstructed Dockerfile with anti-pattern linting.
- 🛡️ **Security hygiene checks** — flags leaked secret files (`.env`, SSH / cloud keys, certs), hardcoded credentials in `ENV`/labels/Dockerfile (key names only, never the value), setuid & world-writable files, and containers that run as root.
- 🤖 **AI optimization report** — hand the full analysis to any OpenAI-compatible model and get a prioritized fix list, plus version-over-version regression detection.
- 🚦 **CI gate** — fail the pipeline when an image drops below your efficiency / wasted-bytes thresholds.
- 🌐 **Terminal · Web · JSON / Markdown · WeCom** — explore interactively, expose an HTTP API, export a report, or push results to a chat.

> **Scope note:** diving focuses on size, structure, and basic security checks (leaked secret files, file permissions, runs-as-root, etc.). It scans file **paths** and image metadata — it does **not** do CVE/vulnerability scanning or file-content scanning. Pair it with Trivy/grype/docker scout for vulnerability coverage.

## Quick start

```bash
# 1. install — pick one:
curl -fsSL https://raw.githubusercontent.com/vicanso/diving-rs/main/install.sh | sh   # prebuilt binary
cargo install diving                                                                  # from crates.io

# 2. dive in
diving redis:alpine
```

That's it — no Docker daemon required. Prebuilt binaries for Linux / macOS / Windows are also on the [release page](https://github.com/vicanso/diving-rs/releases), or build the latest from source with `cargo install --git https://github.com/vicanso/diving-rs`.

Inside the TUI:

| Key | Action |
|-----|--------|
| `1` | Show only `Modified` / `Removed` files of the current layer |
| `2` | Show only files ≥ 1 MB |
| `Esc` / `0` | Reset the view |

## Analyze any image

diving accepts three source types:

```bash
# from a registry (default) — Docker Hub, quay.io, private registries…
diving redis:alpine
diving quay.io/prometheus/node-exporter

# pick an architecture for multi-arch images
diving redis:alpine?arch=arm64

# from the local docker client
diving docker://redis:alpine

# from a saved tar file
diving file:///tmp/redis.tar
```

## Export a report

```bash
# JSON
diving redis:alpine --output-file result.json

# Markdown (detected by the .md extension)
diving redis:alpine --output-file result.md

# Markdown to stdout — base image layers are auto-detected and hidden by default
diving myimage:latest --output-file -

# include the base image layers
diving myimage:latest --output-file - --no-skip-base
```

## CI gate

Run diving in CI to keep images lean. With `CI=true` it prints the efficiency score and **exits `1`** when any threshold is exceeded.

```bash
CI=true diving redis:alpine
```

Thresholds are configurable in `~/.diving/config.yml`:

| Option | Default | Meaning |
|--------|---------|---------|
| `lowest_efficiency` | `0.95` | Minimum acceptable efficiency score (0–1) |
| `highest_wasted_bytes` | `20971520` (20 MB) | Maximum wasted bytes |
| `highest_user_wasted_percent` | `0.1` | Maximum wasted percentage (0–1) |

## AI analysis

Provide an OpenAI-compatible API key and diving sends the full Markdown analysis (layers, reconstructed Dockerfile, wasted space, large files, security findings) to the model and prints a prioritized optimization report instead of opening the TUI. When the `ENTRYPOINT`/`CMD` points to a script inside the image, that script is read from the layers and included, so the model can review what the container actually runs.

```bash
# enable AI analysis (prints the report, skips the TUI)
diving redis:alpine --ai-api-key sk-xxxx

# custom endpoint / model
diving redis:alpine \
  --ai-api-key sk-xxxx \
  --ai-base-url https://your-gateway/v1 \
  --ai-model gpt-4o

# configure via the environment
export OPENAI_API_KEY=sk-xxxx
diving redis:alpine

# control the report language (also affects terminal / Markdown output)
diving redis:alpine --ai-api-key sk-xxxx --lang zh
```

| Flag | Environment | Default | Description |
|------|-------------|---------|-------------|
| `--ai-api-key` | `OPENAI_API_KEY` | — | OpenAI-compatible API key. Providing it enables AI analysis. |
| `--ai-base-url` | `OPENAI_BASE_URL` | `https://api.openai.com/v1` | API base URL. A full `.../chat/completions` URL is also accepted. |
| `--ai-model` | `OPENAI_MODEL` | `gpt-4o` | Model name. |
| `--ai-system-prompt` | `OPENAI_SYSTEM_PROMPT` | built-in DevSecOps template | Override the system prompt to fully replace the built-in one. |
| `--lang` | `DIVING_LANG` | system locale | Output language: `en` or `zh`. |
| `--no-ai-history` | — | off | Skip the regression comparison for this run (the snapshot is still refreshed). |

Each run stores a snapshot under `~/.diving/ai_history/`. On the next run of the same image, the previous snapshot is sent alongside the current one so the model can flag size regressions / bloat between versions. `--no-ai-history` skips that comparison for one run (e.g. when the baseline is stale); the snapshot is still refreshed so subsequent runs compare against this one.

> Security tip: the API key, base URL and webhook are **CLI/env only** — they are never accepted as web query parameters, so they don't end up in access logs.

## WeCom push

Pass a WeCom (企业微信) group-bot webhook to push the result straight into a chat instead of opening the TUI. Content is chosen so it always fits the bot's ~4096-byte markdown limit:

- with `--ai-api-key` set → the concise AI report is pushed
- without AI → a short summary (efficiency score, wasted space, recommendations)

```bash
# bot key (expanded to the standard webhook URL automatically)
diving redis:alpine --wecom-webhook 693a91f6-7aoc-4bc4-97a0-0ec2sifa5aaa

# or the full webhook URL
diving redis:alpine --wecom-webhook "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=KEY"

# push the AI report instead of the summary
diving redis:alpine --ai-api-key sk-xxxx --wecom-webhook KEY

# or from the environment
export WECOM_WEBHOOK=KEY
diving redis:alpine
```

| Flag | Environment | Default | Description |
|------|-------------|---------|-------------|
| `--wecom-webhook` | `WECOM_WEBHOOK` | — | WeCom group-bot webhook URL, or a bare bot key. Providing it pushes the result and skips the TUI. |

Oversized content is truncated to the WeCom limit with a `… (truncated)` marker.

## Web mode

Run diving as an HTTP server with a React frontend for remote analysis.

```bash
# Create the data directory and grant it to the container user (UID/GID 1000)
mkdir -p $PWD/diving
chown -R 1000:1000 $PWD/diving

docker run -d --restart=always \
  -p 7001:7001 \
  -v $PWD/diving:/home/rust/.diving \
  --name diving \
  vicanso/diving
```

Open `http://127.0.0.1:7001/` in the browser.

![](./assets/diving-web.png)

The container runs as a non-root UID (`1000:1000`); the `chown` above lets it write the layer cache (without it the container fails to start). The image is based on `debian:bookworm-slim` with `ca-certificates` and `tzdata`. It ships no `wget`/`curl`, so there is no in-image `HEALTHCHECK` — probe `GET /ping` from your orchestrator (Kubernetes `livenessProbe`, a sidecar, etc.) instead.

Change the listen address with `--listen`:

```bash
diving --mode web --listen 0.0.0.0:8080
```

### API

#### `GET /api/analyze`

Analyze a Docker image and return the result.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `image` | string | yes | Image reference (same formats as terminal mode) |
| `format` | string | no | Set to `markdown` to return a Markdown report instead of JSON |
| `skipBase` | bool | no | When `format=markdown`, auto-detect and hide base image layers (default `true`); set `false` to include them |

```bash
# JSON response (default)
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine"

# specify architecture
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine%3Farch%3Darm64"

# Markdown report
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine&format=markdown"

# Markdown report including base layers (hidden by default)
curl "http://127.0.0.1:7001/api/analyze?image=myimage:latest&format=markdown&skipBase=false"
```

## Sensitive-file scanning

During analysis diving scans every file **path** against built-in rules (`.env` files, SSH private keys, AWS/GCP credentials, TLS private keys, kubeconfig, `.htpasswd`, an accidentally-copied `.git` directory, …) and reports matches under **Security Warnings**. (It scans paths and metadata, not file contents.)

Extend or suppress the rules with `~/.diving/sensitive-files` — one rule per line:

| Line format | Effect |
|-------------|--------|
| `<glob-pattern>` | Flag matching files (reason: "Custom sensitive file") |
| `<glob-pattern> \| <reason>` | Flag with a custom reason label |
| `!<glob-pattern>` | Ignore / suppress matches (overrides built-in and custom patterns above) |

Lines starting with `#` and blank lines are ignored. Globs are case-insensitive; `*` matches across directory separators, and patterns are also tested against the filename alone (so `*.pem` matches `a/b/cert.pem`).

```
# ── Extra patterns ───────────────────────────────────────────
**/*.vault-token | Vault token
**/app-secrets.json | Application secrets

# ── Suppress built-in rules for intentional inclusions ───────
!**/.env.example
!**/.env.template
!**/certs/nginx.crt
!**/testdata/**
!**/fixtures/**
```

## Configuration

Config file: `~/.diving/config.yml`.

| Option | Default | Description |
|--------|---------|-------------|
| `layer_path` | `~/.diving/layers` | Layer blob cache directory |
| `layer_ttl` | `90d` | TTL for cached layer blobs **and** analysis results; an entry is purged if not accessed within this duration |
| `analysis_path` | `~/.diving/analysis` | Analysis-result cache directory |
| `cleanup_interval_hours` | `1` | How often (hours) caches are swept for expired entries |
| `threads` | `min(layers, 2 × CPUs)` | Concurrent layer fetch + decompression tasks. Raise on fast networks with many layers; lower when sharing the host |
| `lowest_efficiency` | `0.95` | CI check — minimum efficiency score (0–1) |
| `highest_wasted_bytes` | `20971520` | CI check — maximum wasted bytes (20 MB) |
| `highest_user_wasted_percent` | `0.1` | CI check — maximum wasted percentage (0–1) |

```yaml
layer_ttl: 30d
cleanup_interval_hours: 6
threads: 4
lowest_efficiency: 0.95
highest_wasted_bytes: 20971520
highest_user_wasted_percent: 0.1
```

## How caching works

diving keeps two on-disk caches under `~/.diving/`, both governed by `layer_ttl` and swept hourly:

- **Layer blobs** (`~/.diving/layers/`) — compressed layer downloads, keyed by layer digest. A hit skips the network download; decompression and file-tree construction still run.
- **Analysis results** (`~/.diving/analysis/`) — the fully analyzed result, keyed by the `Docker-Content-Digest` (from a `HEAD` against the manifest endpoint) plus architecture. A hit short-circuits the entire pipeline.

The analysis cache is **content-addressable**, so re-pushing a mutable tag like `:latest` automatically invalidates the entry. If the `HEAD` probe fails for any reason, diving silently falls back to a full analysis — caching never blocks a request.

> Because layer data is downloaded from the source (e.g. Docker Hub), the first run on a large image can take a while. Interrupted downloads resume automatically. For privately-deployed registries, run diving (or its web image) on a host that can reach the registry.

## License

Licensed under the [Apache License 2.0](./LICENSE).
