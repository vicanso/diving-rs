//! Theme tokens and bundled IBM Plex font registration for `Diving Desktop`.

use nsh_base::{Theme, theme, tokens_of};
use nsh_core::prelude::*;

use crate::domain::AppState;
use crate::i18n::UiLang;
use crate::ui::data::{FileOpKind, LayerOpcodeKind};

const THEME_JSON: &str = include_str!("../../theme.json");

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

/// Registers the offline IBM Plex Mono woff2 faces synchronously before Frame 1.
///
/// UI sans-serif text intentionally uses the platform native font stack (`SF Pro` / `PingFang SC`
/// on macOS).
pub fn register_fonts() {
    let faces: [&[u8]; 3] = [FONT_MONO_REGULAR, FONT_MONO_MEDIUM, FONT_MONO_SEMIBOLD];
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

/// Outer panel / card / dialog corner radius (`radius-lg: 12pt`).
pub fn radius_lg() -> Points {
    number_token("radius-lg", 12.0)
}

/// Standard interactive control & popup corner radius (`radius: 7pt`).
pub fn radius_control() -> Points {
    theme().radius
}

/// Inset nested tag / opcode / checkbox corner radius (`radius-inset: 5pt`).
pub fn radius_inset() -> Points {
    number_token("radius-inset", 5.0)
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

/// Elevated active pill surface for tabs and segmented controls over `muted` tracks.
pub fn surface_elevated() -> AdaptiveColor {
    color_token("surface-elevated", |t| t.popover)
}

/// Unchecked switch track background with crisp contrast on `background` and `card`.
pub fn switch_track() -> AdaptiveColor {
    color_token("switch-track", |t| t.secondary)
}

/// Elevated switch thumb fill visible over both unchecked and checked tracks.
pub fn switch_thumb() -> AdaptiveColor {
    color_token("switch-thumb", |t| t.primary_foreground)
}

/// Captures the ambient `AppState::lang` signal if mounted (falling back to English in `UiGallery`).
pub(crate) fn ambient_lang_signal() -> Option<Signal<UiLang>> {
    use_context::<AppState>().map(|state| state.lang)
}

/// Resolves a localized `(en, zh)` label reactively from an optional ambient language signal.
pub(crate) fn localized_ui_label(
    lang: Option<Signal<UiLang>>,
    en: &'static str,
    zh: &'static str,
) -> SharedString {
    let current = lang.map(|sig| sig.get()).unwrap_or(UiLang::En);
    SharedString::from(match current {
        UiLang::En => en,
        UiLang::Zh => zh,
    })
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

    fn srgb_to_linear(channel: f32) -> f32 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    fn hsla_luminance(c: Hsla) -> f32 {
        let rgba: Rgba = c.into();
        0.2126 * srgb_to_linear(rgba.r)
            + 0.7152 * srgb_to_linear(rgba.g)
            + 0.0722 * srgb_to_linear(rgba.b)
    }

    fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
        let la = hsla_luminance(a);
        let lb = hsla_luminance(b);
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn theme_json_loads_with_zero_ignored_tokens() {
        let (parsed, ignored) = Theme::from_json_lenient(THEME_JSON).expect("valid JSON");
        assert!(ignored.is_empty(), "ignored theme tokens: {ignored:?}");
        assert!(parsed.color("primary-hover").is_some());
        assert!(parsed.color("surface-elevated").is_some());
        assert!(parsed.color("switch-track").is_some());
        assert!(parsed.color("switch-thumb").is_some());
        assert!(parsed.color("treemap-1").is_some());
        assert!(parsed.color("diverge-up").is_some());
        assert!(parsed.number("radius-lg").is_some());
        assert!(parsed.number("radius-inset").is_some());
    }

    #[test]
    fn treemap_slots_meet_wcag_aa_contrast_in_light_and_dark() {
        let (parsed, _) = Theme::from_json_lenient(THEME_JSON).expect("valid JSON");
        for (slot_key, ink_key) in [
            ("treemap-1", "treemap-ink"),
            ("treemap-2", "treemap-ink"),
            ("treemap-3", "treemap-ink"),
            ("treemap-other", "treemap-ink-other"),
        ] {
            let slot = parsed.color(slot_key).expect(slot_key);
            let ink = parsed.color(ink_key).expect(ink_key);
            let light_ratio = contrast_ratio(slot.light, ink.light);
            let dark_ratio = contrast_ratio(slot.dark, ink.dark);
            assert!(
                light_ratio >= 4.5,
                "light {slot_key} vs {ink_key} contrast {light_ratio:.2} < 4.5"
            );
            assert!(
                dark_ratio >= 4.5,
                "dark {slot_key} vs {ink_key} contrast {dark_ratio:.2} < 4.5"
            );
        }
    }

    fn blend_over(fg: Hsla, bg: Hsla) -> Hsla {
        let f: Rgba = fg.into();
        let b: Rgba = bg.into();
        let a = f.a;
        Rgba {
            r: f.r * a + b.r * (1.0 - a),
            g: f.g * a + b.g * (1.0 - a),
            b: f.b * a + b.b * (1.0 - a),
            a: 1.0,
        }
        .into()
    }

    #[test]
    fn semantic_and_tertiary_tokens_meet_wcag_aa_contrast() {
        let (parsed, _) = Theme::from_json_lenient(THEME_JSON).expect("valid JSON");
        let color = |key: &str| parsed.color(key).expect(key);

        let bg = color("background");
        let card = color("card");
        let sidebar = color("sidebar");
        let muted = color("muted");

        for (fg_key, surface_key, surface) in [
            ("text-tertiary", "background", bg),
            ("text-tertiary", "card", card),
            ("text-tertiary", "sidebar", sidebar),
            ("text-tertiary", "muted", muted),
            ("muted-foreground", "background", bg),
            ("muted-foreground", "card", card),
            ("muted-foreground", "sidebar", sidebar),
            ("muted-foreground", "muted", muted),
            ("primary-foreground", "primary", color("primary")),
            (
                "sidebar-primary-foreground",
                "sidebar-primary",
                color("sidebar-primary"),
            ),
            (
                "destructive-foreground",
                "destructive",
                color("destructive"),
            ),
            ("diverge-up", "background", bg),
            ("diverge-up", "card", card),
            ("diverge-up", "muted", muted),
            ("diverge-down", "background", bg),
            ("diverge-down", "card", card),
            ("diverge-down", "muted", muted),
        ] {
            let fg = color(fg_key);
            let light_ratio = contrast_ratio(fg.light, surface.light);
            let dark_ratio = contrast_ratio(fg.dark, surface.dark);
            assert!(
                light_ratio >= 4.5,
                "light {fg_key} on {surface_key} contrast {light_ratio:.2} < 4.5"
            );
            assert!(
                dark_ratio >= 4.5,
                "dark {fg_key} on {surface_key} contrast {dark_ratio:.2} < 4.5"
            );
        }

        for (fg_key, soft_key) in [
            ("success", "success-soft"),
            ("warning", "warning-soft"),
            ("danger", "danger-soft"),
            ("info", "info-soft"),
            ("op-added", "op-added-bg"),
            ("op-modified", "op-modified-bg"),
            ("op-removed", "op-removed-bg"),
        ] {
            let fg = color(fg_key);
            let soft = color(soft_key);
            let light_soft_on_card = blend_over(soft.light, card.light);
            let dark_soft_on_card = blend_over(soft.dark, card.dark);
            let light_soft_on_muted = blend_over(soft.light, muted.light);
            let dark_soft_on_muted = blend_over(soft.dark, muted.dark);
            assert!(
                contrast_ratio(fg.light, light_soft_on_card) >= 4.5,
                "light {fg_key} on {soft_key}/card < 4.5"
            );
            assert!(
                contrast_ratio(fg.dark, dark_soft_on_card) >= 4.5,
                "dark {fg_key} on {soft_key}/card < 4.5"
            );
            assert!(
                contrast_ratio(fg.light, light_soft_on_muted) >= 4.5,
                "light {fg_key} on {soft_key}/muted < 4.5"
            );
            assert!(
                contrast_ratio(fg.dark, dark_soft_on_muted) >= 4.5,
                "dark {fg_key} on {soft_key}/muted < 4.5"
            );
        }
    }
}
