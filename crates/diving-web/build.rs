//! Guarantees the `dist/` directory embedded by `rust_embed`
//! (see `src/dist.rs`, `#[folder = "dist/"]`) exists at compile time.
//!
//! The web frontend lives in `web/` and is compiled into `dist/` by
//! `make build-web` (yarn). That output is `.gitignore`d, so a source-only
//! build — `cargo install diving` from crates.io, `cargo install --git`, or
//! a fresh `git clone` + `cargo build` — has no `dist/`, and the
//! `#[derive(RustEmbed)]` would fail to compile. To keep those paths working
//! without Node/yarn, drop in a minimal placeholder page when no real build
//! is present.
//!
//! This only degrades the React SPA: the terminal UI (the default mode)
//! needs no web assets, and `--mode web`'s JSON API (`/api/analyze`,
//! `/api/file`, `/api/latest-images`) still works — only the bundled UI is
//! the placeholder until a real frontend is embedded.
//!
//! Released crates.io packages bundle the real `dist/` via the `include`
//! key in `Cargo.toml` (populated by `make build-web` before
//! `cargo publish`), so `cargo install diving` ships the full web UI.

use std::env;
use std::fs;
use std::path::Path;

const PLACEHOLDER_HTML: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>diving</title>
    <style>
      body { font-family: system-ui, sans-serif; margin: 4rem auto; max-width: 42rem; padding: 0 1rem; line-height: 1.6; color: #18181b; }
      code { background: #f4f4f5; padding: .15rem .35rem; border-radius: .25rem; }
      a { color: #2563eb; }
    </style>
  </head>
  <body>
    <h1>diving</h1>
    <p>This build does not include the bundled web UI.</p>
    <p>
      The terminal UI works fully — run <code>diving &lt;image&gt;</code> — and
      the JSON API on this server is live (try <code>/api/analyze?image=redis:alpine</code>).
    </p>
    <p>
      To get the web UI: install the released crate with
      <code>cargo install diving</code>, grab a prebuilt binary, or build from
      source with <code>make build-web</code> before <code>cargo build</code>.
    </p>
    <p><a href="https://github.com/vicanso/diving-rs">github.com/vicanso/diving-rs</a></p>
  </body>
</html>
"#;

fn main() {
    // Re-run when the frontend output appears or changes (e.g. after
    // `make build-web`) so the embedded assets stay in sync.
    println!("cargo:rerun-if-changed=dist");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let dist = Path::new(&manifest_dir).join("dist");
    let index = dist.join("index.html");

    // A real `make build-web` output (or a crates.io-bundled `dist/`) is
    // present — embed it untouched.
    if index.exists() {
        return;
    }

    fs::create_dir_all(&dist).expect("failed to create dist/ placeholder directory");
    fs::write(&index, PLACEHOLDER_HTML).expect("failed to write dist/ placeholder index.html");
}
