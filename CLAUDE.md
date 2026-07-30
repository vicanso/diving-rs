# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**diving-rs** is a Rust-based tool for analyzing Docker image layers to understand space usage and inefficiencies. It provides two interfaces:
- **Terminal mode** (default): TUI built with ratatui for interactive layer exploration
- **Web mode**: HTTP server with React frontend for remote analysis

The tool fetches image metadata from registries (Docker Hub, private registries with credentials) or local docker/tar files, decompresses layers, and provides detailed file-level visibility into what's consuming space.

On top of the raw layer/file data, the analysis derives:
- **Recommendations** (`src/recommend.rs`): size / necessity / security cards (pkg caches, dev artifacts, Dockerfile lint, secrets in ENV/labels, setuid files, …), localized via `src/i18n.rs`
- **Sensitive-file detection** (`src/image/sensitive.rs` + user patterns from `~/.diving/sensitive-files`)
- **Cross-layer duplicate detection** (blake3 content hashing; `--no-verify-dup` skips it)
- **ELF runtime-compat probe** (`src/image/elf.rs`): entrypoint libc family / min-glibc vs. base OS
- **AI report** (`src/ai.rs`, enabled by `--ai-api-key` / `$OPENAI_API_KEY`): sends the Markdown analysis (plus the previous snapshot under `~/.diving/ai_history/` for regression comparison) to an OpenAI-compatible endpoint
- **WeCom push** (`src/wecom.rs`, `--wecom-webhook`): sends the AI report or a concise summary to a WeCom group bot

## Build System & Common Commands

Build system uses **Cargo** for Rust and **Make** for orchestration.

### Key Make Targets

```bash
# Web frontend
make build-web          # Build React frontend (builds dist/ directory)
make dev-web            # Watch-based dev with cargo-watch

# Rust compilation
make lint               # Run cargo clippy
make fmt                # Format code with cargo fmt
make release            # Build optimized release binary
make dev                # Run terminal mode with test image (redis:alpine)
make dev-docker         # Run with local docker client
make dev-ci             # Run with CI mode enabled

# Pre-commit setup
make hooks              # Install git hooks (runs fmt + lint before commits)
```

### Pre-commit Hook

The repo includes a pre-commit hook (`hooks/pre-commit`) that runs `make fmt && make lint`. Install it with `make hooks`.

## Code Conventions

### Imports: `use` first, no inline fully-qualified paths

When referencing an item from another module, bring it into scope with a `use`
declaration at the top of the file and then call it by its short name. Do **not**
spell out the fully-qualified path inline at the call site.

```rust
// ✅ preferred — import once, use the short name
use crate::i18n::Lang;
use std::collections::HashSet;

fn build(lang: Lang, seen: &HashSet<String>) { /* ... */ }
```

```rust
// ❌ avoid — fully-qualified path inline
fn build(lang: crate::i18n::Lang, seen: &std::collections::HashSet<String>) { /* ... */ }
```

This applies to `crate::` / `super::` paths and to `std`/external crates alike.
Rare exceptions: disambiguating a genuine name collision, or a one-off reference
where a `use` would be misleading — keep those local and obvious.

## Architecture

### Core Design Pattern

The application follows a **dual-mode runtime**: single binary, dual execution paths determined by the `--mode` argument (terminal or web).

### Data Flow: Image Analysis

1. **Image Source Parsing** (`src/image/image_ref.rs`)
   - Parse image references: `redis:alpine`, `docker://`, `file://`, registry URLs, `?arch=` query
   - Determine source type: registry (default), docker client, or local tar file

2. **Auth & Manifest Resolution** (`src/image/registry_auth.rs` + `src/image/docker.rs` + `src/image/oci_image.rs`)
   - Resolve credentials: CLI/env → `~/.docker/config.json` → docker credential helpers
   - Fetch bearer token, then manifest from Docker V2 registry API (HTTPS); tokens/manifests LRU-cached in memory; 401 mid-analysis triggers one token refresh + retry
   - Handle multi-arch images: select by architecture (`?arch=amd64` query param)
   - Support both Docker Schema 2 and OCI Image Index formats

3. **Layer Decompression & Caching** (`src/image/layer.rs` + `src/store/blob.rs`)
   - Stream compressed layer blobs to disk (resumable via HTTP Range, retry on 429)
   - Blob writes are atomic (same-dir temp file + rename) and verified against the `sha256:` digest before entering the cache
   - Cache locally in `~/.diving/layers/` with TTL (default 90 days)
   - Decompress with gzip/zstd on-demand (streaming, headers only — file contents are not buffered)

4. **File Tree Construction** (`src/image/oci_image.rs`)
   - Extract file metadata from layer tar archives (whiteout / opaque-dir aware, unsafe tar paths rejected)
   - Build hierarchical file tree with operations (added, modified, deleted)
   - Calculate efficiency score: `(1 - wastedSize / totalSize) * 100`

5. **Derived Analyses** (`src/image/docker.rs::analyze`)
   - Sensitive files, heuristic tags, big modified files, base-OS fingerprint, cross-layer duplicates, ELF runtime compat, recommendations
   - Full result cached as JSON under `~/.diving/analysis/` keyed by manifest digest + arch (schema-versioned; see `src/store/analysis_cache.rs`)

6. **Output Rendering**
   - **Terminal**: TUI interface via `src/ui/` modules (layers, files, details)
   - **Web**: JSON API via `src/controller.rs` → React frontend in `web/src/`
   - **Markdown** (`src/markdown.rs`): `-o report.md` / `-o -`, also the payload for AI / WeCom

### Module Structure

- **`src/main.rs`**: Thin binary entry: mimalloc, rustls `ring` provider install, tokio runtime construction
- **`src/lib.rs`**: `Args` (clap CLI), `run()` mode selection (terminal/web), CI gate output, cleanup cron; all modules live here so `cargo test --lib` works
- **`src/image/`**: Core image analysis logic
  - `docker.rs`: Registry API client, blob download (streaming + digest verify + 401 token refresh), the `analyze` pipeline
  - `image_ref.rs`: Image reference parsing (`ImageInfo`)
  - `registry_auth.rs`: Credential resolution (CLI/env, docker config.json, credential helpers)
  - `layer.rs`: Tar extraction, streaming decompress, per-file blake3 hashing, whiteout parsing
  - `oci_image.rs`: OCI spec types, file tree building, cross-layer duplicate detection
  - `sensitive.rs`: Path-based secret / cache / dev-artifact heuristics (plus `has_path_frag` segment matcher)
  - `elf.rs`: Entrypoint ELF probe (goblin): libc family, min glibc vs. base OS
- **`src/recommend.rs`**: Rule engine deriving localized recommendation cards from the analysis result (pure function of `DockerAnalyzeResult`)
- **`src/ai.rs`**: OpenAI-compatible AI report + `~/.diving/ai_history/` snapshots for regression comparison
- **`src/wecom.rs`**: WeCom group-bot markdown push
- **`src/markdown.rs`**: Markdown rendering of the analysis result
- **`src/i18n.rs`**: Rust-side en/zh string catalog (`tr` / `fill`); web has its own catalog in `web/src/i18n/`
- **`src/ui/`**: Terminal UI (ratatui-based)
  - `mod.rs`: Main app event loop, state management
  - `layers.rs`, `files.rs`, `layer_detail.rs`, `image_detail.rs`, `util.rs`: Widget definitions
- **`src/controller.rs`: Web API endpoints**
  - `/api/analyze`: Initiate image analysis (singleflight-deduplicated; `format=markdown`, `includeTree=false`, `noVerifyDup` variants)
  - `/api/file`: Download individual files from layers
  - `/api/latest-images`: Return recent analyses
  - Fallback handler serves static assets from embedded `dist/`
- **`src/store/`**: `blob.rs` (atomic blob I/O, sha256 helpers, TTL cleanup), `analysis_cache.rs` (schema-versioned analysis-result JSON cache)
- **`src/config/`**: Configuration loading from `~/.diving/config.yml` + user sensitive patterns
- **`src/util/`**: Shared `reqwest::Client` singleton, HTTP header helpers
- **`src/error.rs`**: HTTP error types with snafu error handling
- **`src/middleware.rs`**: Axum middleware (access logging, trace ID injection)
- **`src/task_local/`**: Task-local storage for request tracing
- **`src/dist.rs`**: Embedded static asset serving with ETag caching (`build.rs` writes a placeholder `dist/` for source-only builds)

### Web Frontend (`web/`)

- **Framework**: React 19 + TypeScript + Vite + Ant Design (function components + hooks)
- **Entry point**: `web/src/main.tsx` → `web/src/App.tsx`
- **Source layout**:
  - `App.tsx`: state + layout wiring (search flow, layer filter, dark-mode hook)
  - `types.ts`: API/view-model interfaces
  - `analysis.ts`: pure computation — summary derivation, file-tree flattening for the virtualized list
  - `components.tsx`: memoized report cards + `VirtualFileTree` + `SearchBar`
  - `icons.tsx`: inline SVG icons
- **Perf notes**: file-tree rows are `useMemo`-cached and filter changes go through `useDeferredValue`, so keyword typing stays responsive on huge trees; report cards are `memo`-ized
- **Key Components**:
  - Image search with architecture selection (amd64/arm64)
  - Layer navigation and file tree viewer (expandable, filterable)
  - Wasted space summary with file deletion candidates
  - Large modified files detection
  - Dark mode support
- **Build**: `yarn install && yarn build` outputs to `dist/`, embedded in binary via `rust-embed`
- **Internationalization**: `web/src/i18n/` (en.ts, zh.ts)

### Key Abstractions

**ImageInfo**: Parsed image reference (registry, user, name, tag, arch)

**DockerAnalyzeResult**: Complete analysis output with:
- `layers`: Metadata per layer (digest, size, unpack size, command)
- `fileTreeList`: File tree per layer with file metadata (mode, uid/gid, size, operation type)
- `fileSummaryList`: Deleted/modified files for efficiency calculation
- `bigModifiedFileList`: Large files added/modified in upper layers

**FileTreeItem**: Hierarchical file representation with operation type:
- Op 0: Normal (added in this layer)
- Op 1: Removed (whiteout)
- Op 2: Modified (exists in previous layer, modified here)

**Operation Codes** (defined in `src/image/oci_image.rs`):
- Used to mark file changes across layer boundaries
- Terminal and web UIs colorize/filter based on operation type

### Request Flow (Web Mode)

1. Frontend sends `GET /api/analyze?image=redis:alpine&arch=amd64`
2. Backend calls `analyze_docker_image(image_info)` (async)
3. Downloads manifest, fetches layers, builds file trees
4. Returns JSON with full analysis
5. Frontend caches in state, renders layers/files UI
6. File download via `/api/file?digest=<layer_digest>&file=<path>`

### CI Integration

When `CI=true` env var is set:
- Output printed to stdout (efficiency score, wasted bytes)
- Enforces three checks (configurable in `~/.diving/config.yml`):
  1. Minimum efficiency threshold (default 95%)
  2. Maximum wasted bytes (default 20MB)
  3. Maximum wasted percent (default 10%)
- Exits with code 1 on failure
- Can output JSON analysis to file with `-o/--output-file`

## Configuration

Config file: `~/.diving/config.yml`

Example:
```yaml
layer_path: ~/.diving/layers              # Blob cache directory
analysis_path: ~/.diving/analysis          # Analysis-result cache directory
layer_ttl: 90d                             # TTL for both caches
cleanup_interval_hours: 1                  # Cache sweep interval (web mode)
lowest_efficiency: 0.95                    # CI efficiency threshold (0-1)
highest_wasted_bytes: 20971520             # 20MB, CI wasted bytes limit
highest_user_wasted_percent: 0.1           # 10%, CI wasted percent limit
worker_threads: 4                          # Tokio runtime worker count
layer_concurrency: 8                       # Concurrent layer download/decompress per image
threads: 4                                 # Legacy single knob (fallback for the two above)
# Web-mode hardening (all optional; defaults keep prior behavior):
registry_allowlist:                        # Non-empty => /api/analyze only accepts these hosts
  - index.docker.io                        #   (use `local-file` / `local-docker` to allow file:// and docker://)
  - ghcr.io
max_download_file_size: 104857600          # /api/file per-file cap (default 100MB)
max_layer_cache_size: 10737418240          # Layer cache total cap; evicts oldest-accessed blobs
```

Additional user config files under `~/.diving/`:
- `sensitive-files`: extra sensitive-file glob patterns (`pattern | reason`, `!pattern` to suppress)
- `ai_history/`: per-image markdown snapshots used by the AI regression comparison

## Multi-Platform Build

CI/CD via GitHub Actions (`.github/workflows/publish.yml`):
- **Linux (x86_64, aarch64)**: Built in musl Docker container
- **macOS (x86_64, aarch64)**: Native Rust targets
- **Windows (x86_64)**: Native build
- **Docker**: Multi-arch image (linux/amd64, linux/arm64) pushed to Docker Hub

All targets require `make build-web` before Rust compilation.

## Error Handling

- **snafu**: Custom error types with context (see `src/image/docker.rs`, `src/store/blob.rs`)
- **Conversion to HTTP**: Error types implement `From<AppError> for HTTPError`
- **Categories**: Errors tagged (e.g., "docker", "blob") for client debugging

## Performance Considerations

- **Layer Caching**: Blobs cached locally (digest-verified, atomic writes); TTL-based cleanup cron in `src/lib.rs`
- **Analysis Caching**: Full results cached on disk keyed by manifest digest + arch (`src/store/analysis_cache.rs`)
- **Singleflight**: Concurrent web requests for the same image share one analysis (`src/controller.rs`)
- **LRU Cache**: Latest 5 analyzed image names cached in memory (`src/controller.rs`)
- **Compression**: Embedded assets gzip-compressed via `rust-embed`
- **Release Build**: Optimized with LTO, single codegen unit, stripped symbols

## Testing & Development Patterns

- Unit tests live inline (`#[cfg(test)]`) across most modules; run with `cargo test --lib`
- `tests/analyze_local.rs` runs the full analyze pipeline end-to-end over a generated `file://` docker-save fixture (no network); run everything with `cargo test`
- For terminal: `make dev` (redis:alpine), `make dev-docker` (local docker)
- For web: `make dev-web` (Vite dev server) + backend running in another terminal
- CI mode testing: `CI=true make dev` outputs pass/fail checks

## Dependency Notes

- **Async Runtime**: tokio (multi-threaded with signal handling)
- **HTTP Client**: reqwest with rustls (no OpenSSL)
- **JSON**: serde_json
- **Compression**: libflate (gzip), zstd
- **TUI**: ratatui + crossterm for terminal control
- **Web Server**: axum with tower for middleware/timeouts
- **Request Tracing**: tracing + tracing-subscriber
