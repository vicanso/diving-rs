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
- 🌐 **Terminal · Web · MCP · JSON / Markdown · WeCom** — explore interactively, expose an HTTP API or an MCP endpoint for AI agents, export a report, or push results to a chat.

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
# JSON — the full analysis plus efficiencyScore / wastedSize / wastedPercent
diving redis:alpine --output-file result.json

# Markdown (detected by the .md extension)
diving redis:alpine --output-file result.md

# Markdown to stdout — base image layers are auto-detected and hidden by default
diving myimage:latest --output-file -

# include the base image layers
diving myimage:latest --output-file - --no-skip-base
```

## CI gate

Run diving in CI to keep images lean. With `CI=true` it prints the efficiency score and **exits `1`** when any threshold is exceeded (see [exit codes](#exit-codes)).

```bash
CI=true diving redis:alpine
```

Thresholds are configurable in `~/.diving/config.yml`, or in a file you point to with `--config` — handy for keeping the gate next to the code:

```bash
CI=true diving --config .diving.yml myimage:latest
```

| Option | Default | Meaning |
|--------|---------|---------|
| `lowest_efficiency` | `0.95` | Minimum acceptable efficiency score (0–1) |
| `highest_wasted_bytes` | `20971520` (20 MB) | Maximum wasted bytes |
| `highest_user_wasted_percent` | `0.1` | Maximum wasted percentage (0–1) |
| `fail_on_severity` | — (off) | Also fail when any recommendation is at this severity or above: `high`, `medium`, `low` or `info` |

By default the recommendations (leaked secret files, runs-as-root, …) are only printed and do not affect the exit code. Set `fail_on_severity: high` to make findings such as a private key baked into the image fail the pipeline. `medium` and below also count heuristic suggestions (Dockerfile lint, docs/locale files, …), so expect more noise. A misspelled value stops diving at startup instead of silently disabling the check.

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Passed |
| `1` | The image failed the gate (a threshold above, or `fail_on_severity`) |
| `2` | diving itself failed: the image could not be pulled or analyzed, the config is invalid, an AI or WeCom call failed, … |

With `CI=true` the gate also applies when an [AI report](#ai-analysis) or [WeCom push](#wecom-push) is configured: diving sends the report first, then runs the checks.

### Accepting known findings

Every recommendation has a stable id, printed in parentheses in the CI output and available as `id` in the JSON. List the findings you have reviewed and accept under `ignore_recommendations`. They are still printed, marked `ignored`, but no longer count toward `fail_on_severity` (the three thresholds above are unaffected):

```yaml
fail_on_severity: high
ignore_recommendations:
  - secfiles
```

| Id | Recommendation | Severity |
|----|----------------|----------|
| `secfiles` | Potential secrets in image | high |
| `secmeta` | Secrets in image metadata (ENV / labels / Dockerfile) | high |
| `worldread` | World-readable secret files | high |
| `runtimecompat` | Entrypoint binary incompatible with the base image's libc | high (medium for a musl binary on a glibc image) |
| `wasted` | Reclaim wasted space | medium (high above 10% wasted) |
| `crossdup` | Files duplicated across layers | low (medium from 5 MB, high from 50 MB) |
| `pkgcache` | Remove package manager cache | medium |
| `devart` | Exclude development artifacts | medium |
| `runasroot` | Container runs as root | medium |
| `setuid` | setuid/setgid binaries | medium |
| `worldwrite` | World-writable files | medium |
| `oversized` | Oversized layer(s) | low or medium |
| `dflint` | Dockerfile anti-patterns | low or medium |
| `layercount` | Reduce layer count | low |
| `junk` | Editor/OS junk files | low |
| `slimbase` | Consider a slimmer base image | low |
| `buildonly` | Build-only files in runtime image | low |
| `doclocale` | Documentation / man / locale data | low |
| `logtemp` | Log / temp files baked into image | low |
| `toolchain` | Build toolchain present in final image | low |
| `bigfiles` | Large files added in recent layers | info |
| `netreclaim` | Net reclaimable estimate | info |

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

With `CI=true` the message opens with the [CI gate](#ci-gate) verdict — passed, or failed with the checks that failed — so the chat shows the outcome without opening the pipeline.

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

The container runs as a non-root UID (`1000:1000`); the `chown` above lets it write the layer cache (without it the container fails to start). The image is based on `debian:trixie-slim` plus a CA certificate bundle. It has no `tzdata`, so timestamps are in UTC, and it ships no `wget`/`curl`, so there is no in-image `HEALTHCHECK` — probe `GET /ping` from your orchestrator (Kubernetes `livenessProbe`, a sidecar, etc.) instead.

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

### MCP

Web mode also serves an [MCP](https://modelcontextprotocol.io) endpoint at `/mcp` (Streamable HTTP transport), so AI agents such as Claude Code can analyze images themselves:

```bash
claude mcp add --transport http diving http://127.0.0.1:7001/mcp
```

The **MCP** button in the web UI's header shows the same setup, filled in with the address you are browsing: the endpoint, the Claude Code command and a JSON config for other clients, each with a copy button. It also warns when that address would be rejected by the `Host` check below.

| Tool | Returns |
|------|---------|
| `analyze_image` | The Markdown report (same as `format=markdown`) |
| `get_findings` | Findings as JSON: efficiency, wasted bytes, recommendations, sensitive files, duplicates, runtime compatibility, per-layer summary |
| `list_files` | A page of files, filterable by layer, directory, keyword and size |
| `read_file` | One text file from a layer (up to 256 KiB; registry images only) |
| `latest_images` | Recently analyzed images |

Layer numbers are 1-based and match the report. MCP calls share the analysis cache, request deduplication and `registry_allowlist` with `/api/analyze`. A cold analysis can take minutes; when the client sends a progress token, diving emits a progress notification every 10 seconds.

Access control:

- By default `/mcp` only accepts requests whose `Host` is loopback (`localhost`, `127.0.0.1`, `::1`), which blocks DNS-rebinding attacks from web pages. To serve remote clients, pick one:
  - Set a token with `--mcp-token <token>` (or `$DIVING_MCP_TOKEN`). Every request must then carry `Authorization: Bearer <token>`, and the `Host` check is skipped:
    ```bash
    claude mcp add --transport http diving https://diving.example.com/mcp \
      --header "Authorization: Bearer <token>"
    ```
  - Allow your hostnames in `~/.diving/config.yml` with `mcp_allowed_hosts: [diving.example.com]` (`"*"` turns the check off).
- `--no-mcp` turns the endpoint off.

The Docker image listens on `0.0.0.0`, so for remote MCP clients pass the token as an environment variable, e.g. `docker run -e DIVING_MCP_TOKEN=<token> …`. Clients on the same host can use `http://127.0.0.1:7001/mcp` without one.

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

Config file: `~/.diving/config.yml`. Use `--config <file>` (`-c`) or `$DIVING_CONFIG` to read a different file; the flag wins over the environment variable. A file given this way must exist — diving exits with an error instead of falling back to defaults — and is parsed as YAML whatever its extension. Only the config file moves: `sensitive-files`, `ai_history/` and the default cache directories stay under `~/.diving/`.

| Option | Default | Description |
|--------|---------|-------------|
| `layer_path` | `~/.diving/layers` | Layer blob cache directory |
| `layer_ttl` | `90d` | TTL for cached layer blobs **and** analysis results; an entry is purged if not accessed within this duration |
| `analysis_path` | `~/.diving/analysis` | Analysis-result cache directory |
| `cleanup_interval_hours` | `1` | How often (hours) caches are swept for expired entries |
| `layer_concurrency` | `min(layers, 2 × CPUs)` | Concurrent layer fetch + decompression tasks per image. Raise on fast networks with many layers; lower when sharing the host |
| `worker_threads` | number of CPUs | Tokio runtime worker threads. Raise for a web server that handles many requests at once |
| `threads` | — | Legacy single knob: used for both of the two options above when they are not set |
| `lowest_efficiency` | `0.95` | CI check — minimum efficiency score (0–1) |
| `highest_wasted_bytes` | `20971520` | CI check — maximum wasted bytes (20 MB) |
| `highest_user_wasted_percent` | `0.1` | CI check — maximum wasted percentage (0–1) |
| `fail_on_severity` | — | CI check — fail when any recommendation is at this severity or above (`high` / `medium` / `low` / `info`); unset = recommendations never fail the run |
| `ignore_recommendations` | — | CI check — recommendation ids excluded from `fail_on_severity` (see [Accepting known findings](#accepting-known-findings)) |
| `registry_allowlist` | — | Web mode: when non-empty, `/api/analyze` and MCP only accept images from these registry hosts (e.g. `index.docker.io`, `ghcr.io`); add `local-file` / `local-docker` to allow `file://` / `docker://` |
| `max_download_file_size` | `104857600` | Web mode: largest single file `/api/file` will serve (100 MB) |
| `analysis_memory_ttl` | `1m` | Web mode: how long a finished analysis is kept in memory. Requests for the same image within this window are answered without contacting the registry; the price is that a re-pushed tag can take this long to show up. `0s` turns it off |
| `max_concurrent_analyses` | — | Web mode: how many different images may be analyzed at once; further requests wait their turn. Unset = no limit |
| `max_layer_cache_size` | — | Total size cap for the layer cache; when exceeded, the least recently accessed blobs are evicted. Unset = TTL cleanup only |
| `mcp_allowed_hosts` | — | Web mode: extra `Host` values `/mcp` accepts besides loopback (`"*"` turns the check off; ignored when `--mcp-token` is set) |

```yaml
layer_ttl: 30d
cleanup_interval_hours: 6
layer_concurrency: 4
lowest_efficiency: 0.95
highest_wasted_bytes: 20971520
highest_user_wasted_percent: 0.1
```

## How caching works

diving keeps two on-disk caches under `~/.diving/`, both governed by `layer_ttl` and swept hourly:

- **Layer blobs** (`~/.diving/layers/`) — compressed layer downloads, keyed by layer digest. A hit skips the network download; decompression and file-tree construction still run.
- **Analysis results** (`~/.diving/analysis/`) — the fully analyzed result, keyed by the `Docker-Content-Digest` (from a `HEAD` against the manifest endpoint) plus architecture. A hit short-circuits the entire pipeline.

The analysis cache is **content-addressable**, so re-pushing a mutable tag like `:latest` automatically invalidates the entry. If the `HEAD` probe fails for any reason, diving silently falls back to a full analysis — caching never blocks a request.

**When the registry is unavailable.** If the registry cannot be reached, is overloaded (5xx) or rate-limits the request (429), and the same image reference was analyzed before, diving shows that cached analysis instead of an error. The terminal, the Markdown report, the web UI and the JSON (`staleAsOf`) all say when it was made, because the tag may point at a different image by now. An error the registry itself returns (401, 403, 404) is never papered over this way, and neither is a CI run: with `CI=true` the gate has to judge the image as it is, so it fails with exit code `2`.

> Because layer data is downloaded from the source (e.g. Docker Hub), the first run on a large image can take a while. Interrupted downloads resume automatically. For privately-deployed registries, run diving (or its web image) on a host that can reach the registry.

## License

Licensed under the [Apache License 2.0](./LICENSE).
