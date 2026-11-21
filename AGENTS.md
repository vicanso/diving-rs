# AGENTS.md

This file provides guidance to coding agents when working with code in this repository.

## Project Overview

**diving-rs** is a Rust-based workspace for analyzing Docker/OCI image layers to understand space usage, wasted bytes, secrets, duplicates, and runtime compatibility. It produces three core deliverables across a 4-crate Cargo workspace, plus a standalone native desktop GUI:
- **`diving-core`** (`crates/diving-core`, published library): pure Rust async analysis engine (`Analyzer::builder()...build()`) with isolated per-instance caches, singleflight deduplication, registry allowlist, progress callbacks, image diffing, Markdown rendering, and CI gates (no web framework, CLI, TUI, or config-file I/O).
- **`diving-app`** (`crates/diving-app`, published internal helper crate): shared YAML `~/.diving/config.yml` loader, AI report generation, multi-channel outbound notifications (WeCom, Feishu, DingTalk, Slack, generic webhook), logging, and `Analyzer` factories.
- **`diving`** (`crates/diving`, published binary): standalone terminal TUI (ratatui) and CLI / CI gate / `diving diff` tool. Compiles without Node/yarn or web assets.
- **`diving-web`** (`crates/diving-web`, `publish = false`): standalone Axum HTTP server, embedded React SPA (`web/` -> `crates/diving-web/dist/`), MCP `/mcp` endpoint, and inbound `POST /api/webhook` runner packaged in the `vicanso/diving` Docker image.
- **`nsh-app-diving`** (`app/`, excluded from root Cargo workspace): standalone native desktop app (**Diving Desktop**, built with `nsh 0.6.1`) backed directly by `diving-core` & `diving-app`.

The core engine fetches image metadata from registries (Docker Hub, private registries with credentials), a local docker/podman client, or saved images (`docker save` tar, OCI archive tar, OCI layout directory), decompresses layers, and provides detailed file-level visibility into what's consuming space.

On top of the raw layer/file data, the workspace derives:
- **Recommendations** (`diving-core/src/recommend.rs`): size / necessity / security cards (pkg caches, dev artifacts, Dockerfile lint, secrets in ENV/labels, setuid files, …), localized via `diving-core/src/i18n.rs`
- **Sensitive-file detection** (`diving-core/src/sensitive.rs` + optional user patterns loaded by `diving-app`)
- **Cross-layer duplicate detection** (blake3 content hashing; `--no-verify-dup` / `AnalyzeOptions::verify_dup(false)` skips it)
- **ELF runtime-compat probe** (`diving-core/src/image/elf.rs`): entrypoint libc family / min-glibc vs. base OS; also finds unstripped binaries in the final filesystem
- **Image diff** (`diving-core/src/diff.rs`, `diving diff <base> <target>`, `/api/diff`, MCP `diff_images`)
- **AI report** (`diving-app/src/ai.rs`, enabled by `--ai-api-key` / `$OPENAI_API_KEY`): sends the Markdown analysis (plus previous snapshot under `~/.diving/ai_history/`) to an OpenAI-compatible endpoint
- **Multi-channel notifications** (`diving-app/src/{wecom,notify,summary}.rs`): pushes CI/AI summaries and report links to WeCom, Feishu, DingTalk, Slack, and custom JSON webhooks
- **Inbound registry webhook** (`diving-web/src/webhook.rs`, `POST /api/webhook`): parses Harbor/Docker Hub/ACR/Registry v2/generic JSON push events, invalidates tag caches, analyzes asynchronously, and triggers outbound notifications
- **MCP endpoint** (`diving-web/src/mcp.rs`, `/mcp`): exposes the analysis as MCP tools to AI agents

## Build System & Common Commands

Build system uses **Cargo** workspace and **Make** for orchestration.

### Key Make Targets

```bash
# Three build forms
make build-lib          # Build diving-core release library
make build-bin          # Build diving CLI/TUI release binary (no Node/web needed)
make build-web-bin      # Build React frontend into crates/diving-web/dist/ + release diving-web

# Web frontend & dev
make build-web          # Build React frontend into crates/diving-web/dist/
make dev-web            # Watch-based dev for diving-web
make dev                # Run CLI/TUI mode with test image (redis:alpine)
make dev-docker         # Run with local docker client
make dev-ci             # Run with CI mode enabled

# Native Desktop App (Diving Desktop via nsh 0.6.1)
make dev-app            # Run Diving Desktop on macOS (nsh run --target macos)
make test-app           # Run fmt/clippy/Rust tests + nsh check + nsh test in app/
make build-app          # Build desktop nsh runtime artifacts
make package-macos      # Package installable Diving.app bundle (.nsh/out/)

# Quality & release
make lint               # Run cargo clippy --workspace --all-targets -- -D warnings
make fmt                # Format workspace with cargo fmt --all
make test               # Run cargo test --workspace
make release            # Build both diving and diving-web release binaries
make publish            # Publish diving-core -> diving-app -> diving

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

The workspace separates the reusable analysis engine (`diving-core`) from application wiring (`diving-app`) and the two independent runtime binaries (`diving` CLI/TUI and `diving-web` HTTP/MCP/webhook server):
- **`diving-core`** owns an instance-scoped `Analyzer` built via `Analyzer::builder()`. It keeps zero global mutable state (`OnceCell`/`OnceLock`), does zero implicit `~/.diving` filesystem I/O, never prints to stdout/stderr, and defaults to a locked-down server posture (`allow_local_sources(false)`, `use_docker_config(false)`, `CacheMode::Ephemeral`).
- **`diving`** and **`diving-web`** explicitly opt into `~/.diving/config.yml`, `~/.diving/sensitive-files`, persistent caches, local sources, and Docker credentials via `diving-app::config::{build_cli_analyzer, build_web_analyzer}`. `diving-web` still accepts legacy `--mode web` for container compatibility.

### Data Flow: Image Analysis

1. **Image Source Parsing** (`crates/diving-core/src/image/image_ref.rs`)
   - Parse image references: `redis:alpine`, `name@sha256:…`, `docker://`, `podman://`, `file://`, registry URLs, `?arch=` query (also on `file://`, for multi-platform OCI archives / layouts)
   - Determine source type: registry (default), docker / podman client (`docker save` / `podman save`), or a local `docker save` tar / OCI archive / OCI layout directory (`manifest.json` first, else `index.json`)

2. **Auth & Manifest Resolution** (`image/registry_auth.rs`, `image/registry.rs`, `image/oci_image.rs`)
   - Resolve credentials: explicit `RegistryCredentials` -> optional `~/.docker/config.json` -> docker credential helpers
   - Fetch bearer token, then manifest from Docker V2 registry API (HTTPS); per-`Analyzer` LRU caches in memory; 401 mid-analysis triggers one token refresh + retry
   - Support both Docker Schema 2 and OCI Image Index formats

3. **Layer Decompression & Caching** (`image/layer.rs`, `store/blob.rs`)
   - Stream compressed layer blobs to `CacheStore`'s `layer_dir` (resumable via HTTP Range, retry on 429)
   - Blob writes are atomic (same-dir temp file + rename) and verified against the `sha256:` digest before entering the cache

4. **File Tree Construction** (`image/oci_image.rs`, `image/scan.rs`)
   - Extract file metadata from layer tar archives (whiteout / opaque-dir aware, unsafe tar paths rejected)
   - Calculate efficiency score: `(1 - wastedSize / totalSize) * 100`

5. **Derived Analyses** (`image/pipeline.rs`, `recommend.rs`, `sensitive.rs`, `image/elf.rs`)
   - Sensitive files, heuristic tags, big modified files, base-OS fingerprint, cross-layer duplicates, ELF runtime compat, recommendations
   - Full result cached as JSON in `CacheStore`'s `analysis_dir` keyed by manifest digest (+ arch when multi-platform index)

6. **Output Rendering**
   - **Terminal**: TUI interface via `crates/diving/src/ui/`
   - **Web**: JSON API via `crates/diving-web/src/controller.rs` → React frontend in `web/src/`
   - **Markdown** (`crates/diving-core/src/markdown.rs`): `-o report.md` / `-o -`, also the payload for AI / outbound webhooks

### Workspace Structure

- **`crates/diving-core/`**: Pure analysis engine library
  - `analyzer.rs`: `Analyzer`, `AnalyzerBuilder`, `AnalyzeOptions`, `PreviewOptions`, auth-scoped singleflight, `Weak`-ref TTL memory cache, concurrency `Semaphore`, policy enforcement
  - `image/`: split domain modules (`model.rs`, `local.rs`, `scan.rs`, `registry.rs`, `pipeline.rs`, `image_ref.rs`, `registry_auth.rs`, `layer.rs`, `oci_image.rs`, `elf.rs`)
  - `store/`: `CacheStore`, `CacheConfig`, `CacheMode::{Ephemeral, Persistent}`, `blob.rs`, `analysis_cache.rs`, `recent_cache.rs`
  - `diff.rs`, `recommend.rs`, `sensitive.rs`, `markdown.rs`, `gate.rs`, `progress.rs`, `error.rs`, `i18n.rs`, `trace.rs`
- **`crates/diving-app/`**: Shared app wiring
  - `config.rs`: YAML `~/.diving/config.yml` loader returning `Result<LoadedConfig, String>` + `build_http_client`, `build_cli_analyzer`, `build_web_analyzer`
  - `ai.rs`, `notify.rs`, `wecom.rs`, `summary.rs`, `logging.rs`, `env_util.rs`
- **`crates/diving/`**: Standalone CLI / TUI / CI binary (`src/main.rs`, `src/cli.rs`, `src/ui/*`)
- **`crates/diving-web/`**: Standalone HTTP / MCP / Webhook binary + React SPA embed (`build.rs`, `src/{main,lib,state,error,http_util,dist,middleware,controller,mcp,webhook}.rs`)
- **`app/`**: Native Desktop app (`nsh 0.6.1`, `DESIGN.md`, `theme.json`, `nsh.json`):
  - `src/domain/`: pure derivations (`model.rs`, `treemap.rs`), Tokio `DivingService` (`service.rs`), persistent reactive `AppState` (`state.rs`)
  - `src/ui/`: token-backed component library (`theme.rs`, `motion.rs`, `button.rs`, `controls.rs`, `data.rs`, `input.rs`, `overlay.rs`, `tabs.rs`, `code.rs`, `gallery.rs`)
  - `src/features/`: desktop views & overlays (`title_bar.rs`, `landing.rs`, `inspector/{mod,layer_timeline,file_tree,treemap,findings,dockerfile}.rs`, `diff.rs`, `local_picker.rs`, `file_preview_sheet.rs`, `command_palette.rs`, `menu_and_keys.rs`)

### Web Frontend (`web/`)

- **Framework**: React 19 + TypeScript + Vite + Ant Design (function components + hooks)
- **Entry point**: `web/src/main.tsx` → `web/src/App.tsx`
- **Source layout**:
  - `App.tsx`: state + layout wiring (search flow, layer filter, dark-mode hook)
  - `types.ts`: API/view-model interfaces
  - `analysis.ts`: pure computation — summary derivation, file-tree flattening for the virtualized list, final-filesystem tree + squarified layout for the treemap
  - `components.tsx`: memoized report cards + `VirtualFileTree` (with file preview) + `ImageTreemap` (two directory levels, colored by the top three layers) + `SearchBar` + `McpGuide` (header MCP button and setup dialog; shown when `/api/latest-images` reports `mcp.enabled`)
  - `diff.tsx`: `?base=…&target=…` view (`/api/diff`) and the report's **Compare with…** button
  - `icons.tsx`: inline SVG icons
- **Perf notes**: file-tree rows are `useMemo`-cached and filter changes go through `useDeferredValue`, so keyword typing stays responsive on huge trees; report cards are `memo`-ized
- **Key Components**:
  - Image search with architecture selection (amd64/arm64)
  - Layer navigation and file tree viewer (expandable, filterable)
  - Wasted space summary with file deletion candidates
  - Large modified files detection
  - Dark mode support
- **Build**: `yarn install && yarn build` outputs to `web/dist/`, copied into `crates/diving-web/dist/` by `make build-web` and embedded in `diving-web` via `rust-embed`
- **Fonts**: IBM Plex is bundled (`web/src/fonts.css`, latin woff2 from the `@fontsource` packages) — no external font request, so the UI works on networks that cannot reach a font CDN
- **Internationalization**: `web/src/i18n/` (en.ts, zh.ts)

### Key Abstractions

**ImageInfo**: Parsed image reference (registry, user, name, tag, arch)

**DockerAnalyzeResult**: Complete analysis output with:
- `layers`: Metadata per layer (digest, size, unpack size, command)
- `fileTreeList`: File tree per layer with file metadata (mode, uid/gid, size, operation type)
- `fileSummaryList`: Deleted/modified files for efficiency calculation
- `bigModifiedFileList`: Large files added/modified in upper layers

`stale_as_of` on the result is set only for a fallback result served while the registry was unavailable (serialized as `staleAsOf`, never written to the disk cache).

**AnalyzeReport** (`DockerAnalyzeResult::report`): the result flattened together with `efficiencyScore` / `wastedSize` / `wastedPercent`. This is what `-o` JSON and `/api/analyze` serialize; the bare `DockerAnalyzeResult` (analysis cache) does not store those derived numbers.

`fileSummaryList` entries carry `written_in` (the layer that wrote the copy a later layer overwrites / removes), so wasted space reads "written in layer 1, deleted in layer 3"; analyses cached by 0.12.0 lack it. Every user-facing layer number (web, Markdown, MCP) is 1-based.

**FileTreeItem**: Hierarchical file representation with operation type:
- Op 0: Normal (added in this layer)
- Op 1: Removed (whiteout)
- Op 2: Modified (exists in previous layer, modified here)

**Operation Codes** (defined in `crates/diving-core/src/image/oci_image.rs`):
- Used to mark file changes across layer boundaries
- Terminal and web UIs colorize/filter based on operation type

### Request Flow (Web Mode)

1. Frontend sends `GET /api/analyze?image=redis:alpine&arch=amd64`
2. Backend calls `state.analyzer.analyze(&params.image, opts)` (async)
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
- Optional `highest_image_size` (off by default): uncompressed image size cap
- Optional `fail_on_severity` (off by default): fail when any recommendation is at that severity or above; ids in `ignore_recommendations` (`Recommendation::id`, e.g. `runasroot`) are excluded from it
- `diving diff` has its own gate: `highest_size_increase` (off by default) caps target − base uncompressed growth
- `--summary <file>` writes the short Markdown report with the verdict (PR comment / job summary); `action.yml` at the repo root is a composite GitHub Action that installs a release binary, runs the gate (or `diff` when its `base` input is set) and appends that summary to `$GITHUB_STEP_SUMMARY`. GitHub passes no outputs on from a failed step, so the `summary-file` input (a fixed path) is how a later `if: always()` step reads the summary after a failed gate
- The gate is evaluated before anything is sent (`evaluate_gate` in `crates/diving-core/src/gate.rs`) and also applies after an AI report / WeCom push when `CI=true` (outside CI those paths end the run); in CI the WeCom message opens with the verdict
- Exit codes: `1` = image failed the gate, `2` (`EXIT_ERROR`) = diving itself failed (analysis error, bad config, panic hook, failed push)
- Can output JSON analysis to file with `-o/--output-file`

## Configuration

Config file: `~/.diving/config.yml`, or the file given by `--config` / `-c` (wins) or `$DIVING_CONFIG`. A file given that way must exist (startup error otherwise) and is always parsed as YAML. Both `diving` and `diving-web` `main.rs` call `diving_app::config::load_config(args.config_file())` at startup; only `config.yml` moves — `sensitive-files`, `ai_history/` and the default cache dirs stay under `~/.diving/`.

Example:
```yaml
layer_path: ~/.diving/layers              # Blob cache directory
analysis_path: ~/.diving/analysis          # Analysis-result cache directory
layer_ttl: 90d                             # TTL for both caches
cleanup_interval_hours: 1                  # Cache sweep interval (web mode)
lowest_efficiency: 0.95                    # CI efficiency threshold (0-1)
highest_wasted_bytes: 20971520             # 20MB, CI wasted bytes limit
highest_user_wasted_percent: 0.1           # 10%, CI wasted percent limit
highest_image_size: 500MB                  # CI: uncompressed image size cap (unset = off)
highest_size_increase: 50MB                # CI, `diving diff` only: max target − base growth (unset = report only)
fail_on_severity: high                     # CI: also fail on recommendations at/above this severity (unset = off)
ignore_recommendations: [runasroot]        # CI: recommendation ids that don't count toward fail_on_severity
worker_threads: 4                          # Tokio runtime worker count
layer_concurrency: 8                       # Concurrent layer download/decompress per image
threads: 4                                 # Legacy single knob (fallback for the two above)
# Web-mode hardening (all optional; defaults keep prior behavior):
registry_allowlist:                        # Non-empty => /api/analyze, /api/diff and MCP only accept these hosts
  - index.docker.io                        #   (`local-file` / `local-docker` / `local-podman` allow file:// / docker:// / podman://)
  - ghcr.io
max_download_file_size: 104857600          # /api/file per-file cap (default 100MB)
max_layer_cache_size: 10737418240          # Layer cache total cap; evicts oldest-accessed blobs
analysis_memory_ttl: 1m                    # Web: keep finished analyses in memory this long (default 1m, 0s = off)
max_concurrent_analyses: 4                 # Web: cap on different images analyzed at once (unset = unlimited)
mcp_allowed_hosts:                         # Extra Host headers /mcp accepts (default loopback only;
  - diving.example.com                     #   "*" disables; ignored when --mcp-token is set)
```

Additional user config files under `~/.diving/`:
- `sensitive-files`: extra sensitive-file glob patterns (`pattern | reason`, `!pattern` to suppress)
- `ai_history/`: per-image markdown snapshots used by the AI regression comparison

## Multi-Platform Build

CI/CD via GitHub Actions (`.github/workflows/publish.yml`):
- **Linux (`x86_64`, `aarch64`)**: Built in musl Docker container (`cargo build --release -p diving`, no Node/web required)
- **macOS (`x86_64`, `aarch64`)**: Native Rust targets (`diving` CLI)
- **Windows (`x86_64`)**: Native build (`diving` CLI)
- **Docker**: Multi-arch `diving-web` image (`linux/amd64`, `linux/arm64`) pushed to Docker Hub (`vicanso/diving`)

Only Docker web builds (`diving-web`) run `make build-web`.

## Error Handling

- **snafu**: Unified `diving_core::Error` with context and predicates (`is_registry_unavailable`, `is_unauthorized`, `is_not_found`, `is_forbidden`)
- **Conversion to HTTP**: `From<diving_core::Error> for HTTPError` in `crates/diving-web/src/error.rs`
- **Categories**: Errors tagged (`"registry"`, `"docker"`, `"blob"`, `"layer"`) for client debugging

## Performance Considerations

- **Layer Caching**: Blobs cached per `Analyzer` instance (`CacheMode::Persistent` or `CacheMode::Ephemeral`); TTL cleanup via `Analyzer::cleanup_caches()`
- **Analysis Caching**: Full results cached on disk keyed by manifest digest (+ arch for a multi-platform index) (`crates/diving-core/src/store/analysis_cache.rs`)
- **Registry-unavailable fallback**: opt-in via `AnalyzeOptions::fallback_to_last_known(true)` (TUI, web, MCP — not CI gates), surfacing `stale_as_of` when unreachable / 5xx / 429
- **Singleflight & In-memory results**: Built into `Analyzer` (`singleflight`, `memory_cache_ttl`, `max_concurrent_analyses`) keyed cleanly per auth credentials and image identity
- **Response compression**: `tower-http` gzip on the controller routes in `diving-web`; `/mcp` is mounted separately because it streams SSE
- **Release Build**: Workspace `[profile.release]` with LTO, single codegen unit, stripped symbols

## Testing & Development Patterns

- Unit tests live inline (`#[cfg(test)]`) across all 4 crates (`cargo test --workspace`)
- Integration tests in `crates/diving-core/tests/` (`analyze_local`, `offline_fallback`, `analyzer_api`) and `crates/diving-web/tests/` (`mcp_http`, `webhook_push`) run completely isolated with ephemeral `TempDir` caches (never reading/writing `~/.diving`)
- CI (`.github/workflows/test.yml`) verifies `diving-core` & `diving` compile without web assets, then runs `make build-web`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`
- For terminal: `make dev` (`redis:alpine`), `make dev-docker` (`docker://redis:alpine`), `CI=true make dev`
- For web: `make dev-web` (backend) + `cd web && yarn dev` (Vite dev server)

## Dependency Notes

- **Async Runtime**: tokio (multi-threaded with signal handling)
- **HTTP Client**: reqwest with rustls (no OpenSSL)
- **JSON**: serde_json
- **Compression**: flate2 on the pure-Rust `zlib-rs` backend (gzip; ~30% less CPU than the default miniz_oxide on large layers), zstd
- **No `regex`**: the two patterns the project needed (the `WWW-Authenticate` challenge and `exec` lines in entrypoint scripts) are parsed by hand; the crate cost ~460 KiB of binary
- **TUI**: ratatui + crossterm for terminal control
- **Web Server**: axum with tower for middleware/timeouts
- **Request Tracing**: tracing + tracing-subscriber
