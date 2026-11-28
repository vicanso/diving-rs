//! Guarantees the `dist/` directory embedded by `rust_embed`
//! (see `src/dist.rs`, `#[folder = "dist/"]`) exists at compile time.
//!
//! The web frontend lives in `web/` and is compiled into `dist/` by
//! `make build-web` (yarn). That output is `.gitignore`d, so a source-only
//! `cargo build` without running `make build-web` has no `dist/`, and
//! `#[derive(RustEmbed)]` would fail to compile. Drop in a minimal
//! placeholder page when no real frontend build is present.
//!
//! This only affects the embedded React SPA: the JSON API (`/api/analyze`,
//! `/api/diff`, `/api/file`, `/api/latest-images`), `/mcp`, and
//! `POST /api/webhook` still work until `make build-web` embeds the real UI.
//!
//! `diving-web` is not published to crates.io (`publish = false`); release
//! binaries (`make build-web-bin`) and the `vicanso/diving` Docker image run
//! `make build-web` beforehand so the full SPA is bundled.

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
      The JSON API on this server is live (try <code>/api/analyze?image=redis:alpine</code>),
      along with <code>/mcp</code> and <code>POST /api/webhook</code>.
    </p>
    <p>
      To include the web UI, run <code>make build-web</code> before building
      <code>diving-web</code> (or use <code>make build-web-bin</code> / the
      <code>vicanso/diving</code> Docker image).
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
