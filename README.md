# diving-rs

[中文](./README-zh.md)

Exploring each layer in a docker image, it's fast and simple, developed with Rust. There are two modes: terminal(default) and web. 

It does not depend on anything, including docker client.

It supports multiple platforms: linux, windows and macos, you can get it from [release page](https://github.com/vicanso/diving-rs/releases).

Note: Since the layer data needs to be downloaded from the source, such as Docker Hub, it may take a long time. Interrupted downloads of large layers are retried automatically and resume from where they stopped; if it still fails, please try again. It is recommended that the download program be executed locally. For image sources deployed privately, you can deploy the image of Diving on a machine that can access the image source.


## Installation

```bash
curl -fsSL https://raw.githubusercontent.com/vicanso/diving-rs/main/install.sh | sh
```


## config

The config file is `~/.diving/config.yml`, the options:

- `layer_path`: The path of layer cache, default is `~/.diving/layers`
- `layer_ttl`: The TTL of cached layer blobs, default is `90d`. A layer is purged if it has not been accessed for the specified duration
- `cleanup_interval_hours`: How often (in hours) the layer cache is scanned for expired entries, default is `1`
- `threads`: Number of threads for parallel layer downloads, default is the number of logical CPUs
- `lowest_efficiency`: CI check — minimum acceptable efficiency score (0–1), default is `0.95`
- `highest_wasted_bytes`: CI check — maximum wasted bytes, default is `20971520` (20 MB)
- `highest_user_wasted_percent`: CI check — maximum wasted percentage (0–1), default is `0.1`

Example `~/.diving/config.yml`:

```yaml
layer_ttl: 30d
cleanup_interval_hours: 6
threads: 4
lowest_efficiency: 0.95
highest_wasted_bytes: 20971520
highest_user_wasted_percent: 0.1
```

## sensitive-files

During analysis, diving scans every file path against a set of built-in rules (`.env` files, SSH private keys, AWS credentials, TLS certificates, etc.) and reports matches in the analysis output under **Security Warnings**.

You can extend or suppress these checks by creating `~/.diving/sensitive-files`. Each line is one rule:

| Line format | Effect |
|-------------|--------|
| `<glob-pattern>` | Flag matching files as sensitive (reason: "Custom sensitive file") |
| `<glob-pattern> \| <reason>` | Flag with a custom reason label |
| `!<glob-pattern>` | Ignore / suppress matches (overrides both built-in rules and custom patterns above) |

Lines starting with `#` and blank lines are ignored. Glob patterns are case-insensitive; `*` matches across directory separators, and patterns are also tested against the filename alone, so `*.pem` matches `a/b/cert.pem`.

Example `~/.diving/sensitive-files`:

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

## terminal

Supports three data source modes analyze image. The specific form is as follows:

- `registry` get image form docker registry or other registry
- `docker` get image from local docker client
- `file` get image for tar file

```bash
diving redis:alpine

# specify architecture
diving redis:alpine?arch=arm64

diving quay.io/prometheus/node-exporter

diving docker://redis:alpine

diving file:///tmp/redis.tar

# CI mode — prints efficiency score and exits with code 1 if checks fail
CI=true diving redis:alpine

# save analysis result to a JSON file
diving redis:alpine --output-file result.json

# save analysis result as Markdown (detected by .md extension)
diving redis:alpine --output-file result.md

# print Markdown to stdout (all layers shown by default)
diving myimage:latest --output-file -

# add --skip-base to auto-detect and hide base image layers via timestamp gap
diving myimage:latest --output-file - --skip-base
```

- `Current Layer Contents` only show the files of current layer
- `Press 1` only show the `Modified/Removed` files of current layer
- `Press 2` only show the files >= 1MB
- `Press Esc or 0` reset the view mode

![](./assets/diving-terminal.gif)

## AI analysis

Provide an OpenAI-compatible API key to get an AI-generated optimization report instead of the interactive TUI. diving sends the full Markdown analysis (layers, reconstructed Dockerfile, wasted space, large files, security findings) to the model and prints its diagnosis to stdout.

```bash
# enable AI analysis (prints the report, skips the TUI)
diving redis:alpine --ai-api-key sk-xxxx

# custom OpenAI-compatible endpoint and model
diving redis:alpine \
  --ai-api-key sk-xxxx \
  --ai-base-url https://your-gateway/v1 \
  --ai-model gpt-4o

# key / endpoint / model can also come from the environment
export OPENAI_API_KEY=sk-xxxx
diving redis:alpine

# control the report language (also affects terminal/Markdown output)
diving redis:alpine --ai-api-key sk-xxxx --lang zh
```

| Flag | Environment | Default | Description |
|------|-------------|---------|-------------|
| `--ai-api-key` | `OPENAI_API_KEY` | — | OpenAI-compatible API key. Providing it enables AI analysis. |
| `--ai-base-url` | `OPENAI_BASE_URL` | `https://api.openai.com/v1` | API base URL. A full `.../chat/completions` URL is also accepted. |
| `--ai-model` | `OPENAI_MODEL` | `gpt-4o` | Model name. |
| `--lang` | `DIVING_LANG` | system locale | Output language: `en` or `zh`. |

Each run stores a snapshot of the analysis under `~/.diving/ai_history/`. On the next run of the same image, the previous snapshot is sent alongside the current one so the model can flag size regressions / bloat between versions.

## web

```bash
# Create the data directory and grant ownership to the container user (UID/GID 1000)
mkdir -p $PWD/diving
chown -R 1000:1000 $PWD/diving

docker run -d --restart=always \
  -p 7001:7001 \
  -v $PWD/diving:/home/rust/.diving \
  --name diving \
  vicanso/diving
```

The container runs as user `rust` (UID 1000, GID 1000), not root. The `chown` command above grants ownership of the host directory to that user. Without it the container cannot write layer cache files and will fail to start.

To change the listen address, pass `--listen`:

```bash
diving --mode web --listen 0.0.0.0:8080
```

Open `http://127.0.0.1:7001/` in the browser.

![](./assets/diving-web.png)

### API

#### `GET /api/analyze`

Analyze a Docker image and return the result.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `image` | string | yes | Image reference (same formats as terminal mode) |
| `format` | string | no | Set to `markdown` to return a Markdown report instead of JSON |
| `skipBase` | bool | no | When `format=markdown`, auto-detect and hide base image layers via timestamp gap |

**Examples:**

```bash
# JSON response (default)
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine"

# specify architecture
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine%3Farch%3Darm64"

# Markdown report
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine&format=markdown"

# Markdown report with base layers hidden
curl "http://127.0.0.1:7001/api/analyze?image=myimage:latest&format=markdown&skipBase=true"
```