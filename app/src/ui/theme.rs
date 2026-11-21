//! Theme tokens and bundled IBM Plex font registration for `Diving Desktop`.

use nsh_base::{Theme, theme, tokens_of};
use nsh_core::prelude::*;

use crate::ui::data::{FileOpKind, LayerOpcodeKind};

const THEME_JSON: &str = include_str!("../../theme.json");

const FONT_SANS_REGULAR: &[u8] =
    include_bytes!("../../assets/fonts/ibm-plex-sans-latin-400-normal.woff2");
const FONT_SANS_MEDIUM: &[u8] =
    include_bytes!("../../assets/fonts/ibm-plex-sans-latin-500-normal.woff2");
const FONT_SANS_SEMIBOLD: &[u8] =
    include_bytes!("../../assets/fonts/ibm-plex-sans-latin-600-normal.woff2");
const FONT_SANS_BOLD: &[u8] =
    include_bytes!("../../assets/fonts/ibm-plex-sans-latin-700-normal.woff2");
const FONT_MONO_REGULAR: &[u8] =
    include_bytes!("../../assets/fonts/ibm-plex-mono-latin-400-normal.woff2");
const FONT_MONO_MEDIUM: &[u8] =
    include_bytes!("../../assets/fonts/ibm-plex-mono-latin-500-normal.woff2");
const FONT_MONO_SEMIBOLD: &[u8] =
    include_bytes!("../../assets/fonts/ibm-plex-mono-latin-600-normal.woff2");

/// Parses `app/theme.json` into NSH `Tokens` for `cargo run` and headless tests.
pub fn app_tokens() -> Tokens {
    tokens_of(Some(THEME_JSON)).expect("app/theme.json must parse cleanly")
}

/// Registers the offline IBM Plex Sans & Mono woff2 faces synchronously before Frame 1.
pub fn register_fonts() {
    let faces: [&[u8]; 7] = [
        FONT_SANS_REGULAR,
        FONT_SANS_MEDIUM,
        FONT_SANS_SEMIBOLD,
        FONT_SANS_BOLD,
        FONT_MONO_REGULAR,
        FONT_MONO_MEDIUM,
        FONT_MONO_SEMIBOLD,
    ];
    for bytes in faces {
        drop(register_font(bytes));
    }
}

fn color_token(key: &str, fallback: impl FnOnce(&Theme) -> AdaptiveColor) -> AdaptiveColor {
    let current = theme();
    current.color(key).unwrap_or_else(|| fallback(&current))
}

fn number_token(key: &str, fallback: f32) -> Points {
    px(theme().number(key).unwrap_or(fallback))
}

/// Outer panel / card / dialog corner radius (`radius-lg: 10pt`).
pub fn radius_lg() -> Points {
    number_token("radius-lg", 10.0)
}

/// Standard interactive control & popup corner radius (`radius: 6pt`).
pub fn radius_control() -> Points {
    theme().radius
}

/// Inset nested tag / opcode / checkbox corner radius (`radius-inset: 4pt`).
pub fn radius_inset() -> Points {
    number_token("radius-inset", 4.0)
}

/// Configured monospace font family name if set in `theme.json`.
pub fn font_mono_family() -> SharedString {
    theme()
        .font_mono
        .clone()
        .unwrap_or_else(|| SharedString::from("IBM Plex Mono"))
}

pub fn primary_hover() -> AdaptiveColor {
    color_token("primary-hover", |t| t.primary)
}

pub fn primary_active() -> AdaptiveColor {
    color_token("primary-active", |t| t.primary)
}

pub fn secondary_hover() -> AdaptiveColor {
    color_token("secondary-hover", |t| t.secondary)
}

pub fn secondary_active() -> AdaptiveColor {
    color_token("secondary-active", |t| t.secondary)
}

pub fn destructive_hover() -> AdaptiveColor {
    color_token("destructive-hover", |t| t.destructive)
}

pub fn destructive_active() -> AdaptiveColor {
    color_token("destructive-active", |t| t.destructive)
}

pub fn destructive_foreground() -> AdaptiveColor {
    color_token("destructive-foreground", |t| t.primary_foreground)
}

pub fn surface_active() -> AdaptiveColor {
    color_token("surface-active", |t| t.accent)
}

pub fn border_strong() -> AdaptiveColor {
    color_token("border-strong", |t| t.border)
}

pub fn text_tertiary() -> AdaptiveColor {
    color_token("text-tertiary", |t| t.muted_foreground)
}

pub fn success() -> AdaptiveColor {
    color_token("success", |t| t.primary)
}

pub fn success_soft() -> AdaptiveColor {
    color_token("success-soft", |t| t.muted)
}

pub fn warning() -> AdaptiveColor {
    color_token("warning", |t| t.destructive)
}

pub fn warning_soft() -> AdaptiveColor {
    color_token("warning-soft", |t| t.muted)
}

pub fn danger() -> AdaptiveColor {
    color_token("danger", |t| t.destructive)
}

pub fn danger_soft() -> AdaptiveColor {
    color_token("danger-soft", |t| t.muted)
}

pub fn info() -> AdaptiveColor {
    color_token("info", |t| t.primary)
}

pub fn info_soft() -> AdaptiveColor {
    color_token("info-soft", |t| t.muted)
}

pub fn op_color(kind: FileOpKind) -> AdaptiveColor {
    match kind {
        FileOpKind::Added => color_token("op-added", |t| t.primary),
        FileOpKind::Modified => color_token("op-modified", |t| t.destructive),
        FileOpKind::Removed => color_token("op-removed", |t| t.destructive),
    }
}

pub fn op_bg(kind: FileOpKind) -> AdaptiveColor {
    match kind {
        FileOpKind::Added => color_token("op-added-bg", |t| t.muted),
        FileOpKind::Modified => color_token("op-modified-bg", |t| t.muted),
        FileOpKind::Removed => color_token("op-removed-bg", |t| t.muted),
    }
}

pub fn opcode_color(kind: LayerOpcodeKind) -> AdaptiveColor {
    let key = match kind {
        LayerOpcodeKind::From => "opcode-from",
        LayerOpcodeKind::Run => "opcode-run",
        LayerOpcodeKind::Copy => "opcode-copy",
        LayerOpcodeKind::Cmd => "opcode-cmd",
        LayerOpcodeKind::Meta => "opcode-meta",
    };
    color_token(key, |t| t.muted_foreground)
}

pub fn opcode_bg(kind: LayerOpcodeKind) -> AdaptiveColor {
    let key = match kind {
        LayerOpcodeKind::From => "opcode-from-bg",
        LayerOpcodeKind::Run => "opcode-run-bg",
        LayerOpcodeKind::Copy => "opcode-copy-bg",
        LayerOpcodeKind::Cmd => "opcode-cmd-bg",
        LayerOpcodeKind::Meta => "opcode-meta-bg",
    };
    color_token(key, |t| t.muted)
}

pub fn size_bar_track() -> AdaptiveColor {
    color_token("size-bar-track", |t| t.muted)
}

pub fn size_bar_fill(ratio: f32) -> AdaptiveColor {
    if ratio >= 0.75 {
        color_token("size-bar-extreme", |t| t.destructive)
    } else if ratio >= 0.35 {
        color_token("size-bar-heavy", |t| t.primary)
    } else {
        color_token("size-bar-fill", |t| t.primary)
    }
}

pub fn treemap_slot(slot: usize) -> AdaptiveColor {
    let key = match slot {
        1 => "treemap-1",
        2 => "treemap-2",
        3 => "treemap-3",
        _ => "treemap-other",
    };
    color_token(key, |t| t.muted)
}

pub fn treemap_ink(is_other: bool) -> AdaptiveColor {
    if is_other {
        color_token("treemap-ink-other", |t| t.foreground)
    } else {
        color_token("treemap-ink", |t| t.foreground)
    }
}

pub fn diverge_up() -> AdaptiveColor {
    color_token("diverge-up", |t| t.destructive)
}

pub fn diverge_down() -> AdaptiveColor {
    color_token("diverge-down", |t| t.primary)
}

/// Modal backdrop scrim computed cleanly over `black()` without raw hex literals.
pub fn backdrop_scrim() -> Hsla {
    black().opacity(0.45)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_json_loads_with_zero_ignored_tokens() {
        let (parsed, ignored) = Theme::from_json_lenient(THEME_JSON).expect("valid JSON");
        assert!(ignored.is_empty(), "ignored theme tokens: {ignored:?}");
        assert!(parsed.color("primary-hover").is_some());
        assert!(parsed.color("treemap-1").is_some());
        assert!(parsed.color("diverge-up").is_some());
        assert!(parsed.number("radius-lg").is_some());
        assert!(parsed.number("radius-inset").is_some());
    }
}
