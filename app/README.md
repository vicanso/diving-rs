# Diving Desktop

Pure-Rust native desktop application built with [NSH](https://nsh.dev) (`0.6.1`) on top of `diving-core` and `diving-app`. Its UI root is `App` (`src/lib.rs`), declared in `nsh.json`.

## Architecture

- `src/domain/`: Pure image/layer/treemap derivations (`model.rs`, `treemap.rs`), multi-threaded Tokio `DivingService` bridge (`service.rs`), and reactive `AppState` with `nsh_core::storage` persistence (`state.rs`).
- `src/ui/`: Tokenized design system built over `nsh_base` and `theme.json` (`theme.rs`, `motion.rs`, `icons.rs`, `button.rs`, `input.rs`, `controls.rs`, `tabs.rs`, `overlay.rs`, `data.rs`, `code.rs`, `gallery.rs`).
- `src/features/`: Desktop views & overlays composed strictly from `crate::ui` and `nsh_core` primitives (`sidebar.rs`, `title_bar.rs`, `landing.rs`, `inspector/`, `diff.rs`, `local_picker.rs`, `file_preview_sheet.rs`, `command_palette.rs`, `menu_and_keys.rs`).
- `src/i18n.rs`: Synchronized EN / ZH catalog matching `diving_core::Lang`.

## Commands

Run from `app/` (or use `make dev-app` / `make test-app` / `make build-app` / `make package-macos` at repo root):

```sh
nsh run --target macos               # Launch Diving Desktop natively on macOS
nsh check                            # Cargo check + TypeScript test typecheck
nsh build                            # Build the NSH runtime manifest
cargo clippy --all-targets -- -D warnings
cargo test                           # Unit tests + headless Rust E2E tests
nsh test                             # Full Rust + TypeScript headless suite
nsh test gallery.test.tsx --target macos -v   # Render real macOS screenshots into .nsh/out/
nsh package --target macos           # Build ad-hoc signed .app bundle into .nsh/out/
```

## Keyboard Shortcuts

`Mod` maps to `⌘ Command` on macOS and `Ctrl` on Windows/Linux:

| Shortcut | Action |
| --- | --- |
| `Mod + \` | Toggle collapsible left sidebar |
| `Mod + N` | Return to New Analysis launcher |
| `Mod + K` | Open / close Command Palette |
| `Mod + L` | Open / focus inline image switcher |
| `Mod + O` | Open local tar / OCI archive (`file://`) |
| `Mod + Shift + O` | Browse local Docker / Podman images |
| `Mod + D` | Open `Compare with…` diff dialog |
| `Mod + R` | Force re-analyze bypassing in-memory cache |
| `Mod + E` | Export analysis or diff report as Markdown |
| `Mod + 1` ~ `4` | Switch Inspector tabs (`File Tree`, `Treemap`, `Findings`, `Dockerfile`) |
| `/` | Focus layer file filter input (when not editing) |
| `↑` / `↓` | Step selected layer in Inspector (when not editing) |
| `Esc` | Dismiss active dialog/sheet/palette or clear file filter |

NSH-generated artifacts remain under `.nsh/` (gitignored).
