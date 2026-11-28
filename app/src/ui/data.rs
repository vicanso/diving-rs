//! Flat hairline panels, semantic badges, opcode/op pills, size bars, skeletons, and splits.

use nsh_base::{h_resizable, progress, resizable_panel, theme};
use nsh_core::prelude::*;

use crate::ui::icons::{IconName, ui_icon};
use crate::ui::motion::{FAST, dur};
use crate::ui::theme::{
    border_strong, danger, danger_soft, font_mono_family, info, info_soft, op_bg, op_color,
    opcode_bg, opcode_color, radius_inset, radius_lg, size_bar_fill, size_bar_track, success,
    success_soft, treemap_slot, warning, warning_soft,
};

/// Semantic tone for `ui_badge`. Defaults to `Neutral`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeTone {
    #[default]
    Neutral,
    Success,
    Warning,
    Danger,
    Info,
}

/// Dockerfile instruction categories matching `DESIGN.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerOpcodeKind {
    From,
    Run,
    Copy,
    Cmd,
    Meta,
}

impl LayerOpcodeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::From => "FROM",
            Self::Run => "RUN",
            Self::Copy => "COPY",
            Self::Cmd => "CMD",
            Self::Meta => "META",
        }
    }
}

/// File operation categories, always paired with `+`, `~`, or `−` symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOpKind {
    Added,
    Modified,
    Removed,
}

impl FileOpKind {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Added => "+",
            Self::Modified => "~",
            Self::Removed => "−",
        }
    }

    pub fn accessible_label(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Modified => "Modified",
            Self::Removed => "Removed",
        }
    }
}

fn build_panel(
    title: Option<Reactive<SharedString>>,
    header_extra: Option<AnyElement>,
    content: impl IntoElement,
) -> impl IntoElement {
    view()
        .rounded(radius_lg)
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .overflow_hidden()
        .when(title.is_some() || header_extra.is_some(), |el| {
            el.child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_3p5()
                    .py_2()
                    .border_b_1()
                    .border_color(move || theme().border)
                    .when_some(title, |row, heading| {
                        row.child(
                            text(heading)
                                .role(AccessibilityRole::Header)
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().card_foreground),
                        )
                    })
                    .when_some(header_extra, |row, extra| row.child(extra)),
            )
        })
        .child(view().p_3p5().child(content))
}

/// Flat hairline container panel (no drop shadows; single outer radius).
pub fn ui_panel(
    title: Option<SharedString>,
    header_extra: Option<AnyElement>,
    content: impl IntoElement,
) -> impl IntoElement {
    build_panel(
        title.map(IntoReactive::into_reactive),
        header_extra,
        content,
    )
}

/// Flat hairline container panel with a reactive heading label (for localized sections).
pub fn ui_panel_titled(
    title: impl IntoReactive<SharedString>,
    header_extra: Option<AnyElement>,
    content: impl IntoElement,
) -> impl IntoElement {
    build_panel(Some(title.into_reactive()), header_extra, content)
}

fn badge_fg(tone: BadgeTone) -> AdaptiveColor {
    match tone {
        BadgeTone::Neutral => theme().muted_foreground,
        BadgeTone::Success => success(),
        BadgeTone::Warning => warning(),
        BadgeTone::Danger => danger(),
        BadgeTone::Info => info(),
    }
}

fn badge_bg(tone: BadgeTone) -> AdaptiveColor {
    match tone {
        BadgeTone::Neutral => theme().muted,
        BadgeTone::Success => success_soft(),
        BadgeTone::Warning => warning_soft(),
        BadgeTone::Danger => danger_soft(),
        BadgeTone::Info => info_soft(),
    }
}

/// Small tonal semantic badge without heavy outer borders.
pub fn ui_badge(label: impl IntoReactive<SharedString>, tone: BadgeTone) -> impl IntoElement {
    view()
        .px_1p5()
        .py_0p5()
        .rounded(radius_inset)
        .bg(move || badge_bg(tone))
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || badge_fg(tone)),
        )
}

/// Monospace Dockerfile instruction pill (`FROM` / `RUN` / `COPY` / `CMD` / `META`).
pub fn ui_opcode_pill(kind: LayerOpcodeKind) -> impl IntoElement {
    view()
        .w(px(42.0))
        .py_0p5()
        .items_center()
        .justify_center()
        .rounded(radius_inset)
        .bg(move || opcode_bg(kind))
        .child(
            text(kind.label())
                .text_xs()
                .font_family(font_mono_family())
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || opcode_color(kind)),
        )
}

/// File operation badge encoding state with both symbol (`+`/`~`/`−`) and semantic color.
pub fn ui_status_badge(kind: FileOpKind) -> impl IntoElement {
    view()
        .w(px(22.0))
        .py_0p5()
        .items_center()
        .justify_center()
        .rounded(radius_inset)
        .bg(move || op_bg(kind))
        .label(kind.accessible_label())
        .child(
            text(kind.symbol())
                .text_xs()
                .font_family(font_mono_family())
                .font_weight(FontWeight::BOLD)
                .text_color(move || op_color(kind)),
        )
}

fn render_size_bar<F>(ratio: f32, width: Points, fill: F) -> impl IntoElement
where
    F: Fn() -> AdaptiveColor + 'static,
{
    let clamped = ratio.clamp(0.0, 1.0);
    view()
        .w(width)
        .h(px(6.0))
        .rounded_full()
        .bg(size_bar_track)
        .overflow_hidden()
        .accessibility_elements_hidden(true)
        .child(view().h_full().w(relative(clamped)).rounded_full().bg(fill))
}

/// Horizontal proportional micro-bar used in LayerTimeline and FileTree rows.
pub fn ui_size_bar(ratio: f32, width: Points) -> impl IntoElement {
    let clamped = ratio.clamp(0.0, 1.0);
    render_size_bar(clamped, width, move || size_bar_fill(clamped))
}

/// Horizontal proportional micro-bar tinted with the matching Treemap layer slot color.
pub fn ui_treemap_size_bar(ratio: f32, width: Points, color_slot: usize) -> impl IntoElement {
    render_size_bar(ratio, width, move || treemap_slot(color_slot))
}

/// Single hairline horizontal separator owned by its container.
pub fn ui_separator() -> impl IntoElement {
    view()
        .h(px(1.0))
        .w_full()
        .bg(move || theme().border)
        .accessibility_elements_hidden(true)
}

/// Centered empty-state panel with icon, message, and optional recovery action.
pub fn ui_empty_state(
    icon: IconName,
    title: impl IntoReactive<SharedString>,
    description: impl IntoReactive<SharedString>,
    action: Option<AnyElement>,
) -> impl IntoElement {
    view()
        .flex_col()
        .items_center()
        .justify_center()
        .p_8()
        .gap_3()
        .child(
            view()
                .size(px(40.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(move || theme().muted)
                .child(ui_icon(icon, px(20.0), move || theme().muted_foreground)),
        )
        .child(
            text(title)
                .text_center()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(description)
                .text_center()
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .when_some(action, |col, btn| col.child(btn))
}

/// Static placeholder block while genuinely awaiting content.
pub fn ui_skeleton(width: Points, height: Points) -> impl IntoElement {
    view()
        .w(width)
        .h(height)
        .rounded(radius_inset)
        .bg(move || theme().muted)
        .accessibility_elements_hidden(true)
}

/// Determinate or indeterminate linear progress bar built over `nsh_base::progress`.
pub fn ui_progress_bar(
    value: impl IntoReactive<f32>,
    indeterminate: impl IntoReactive<bool>,
    accessibility_label: impl Into<SharedString>,
) -> impl IntoElement {
    let value = value.into_reactive();
    let indeterminate = indeterminate.into_reactive();
    progress()
        .value(move || value.get())
        .indeterminate(move || indeterminate.get())
        .accessibility_label(accessibility_label.into())
        .w_full()
        .track_style(|s| {
            s.h(px(6.0))
                .w_full()
                .rounded_full()
                .bg(size_bar_track)
                .overflow_hidden()
        })
        .indicator_style(|s| {
            s.h_full()
                .rounded_full()
                .bg(move || theme().primary)
                .transition_all()
                .duration(move || dur(FAST))
        })
}

/// Two-pane horizontal resizable workspace split with themed hairline divider.
pub fn ui_resizable_split(
    left_initial: Points,
    left_min: Points,
    left_max: Points,
    left_pane: impl IntoElement,
    right_pane: impl IntoElement,
    on_resize: impl Fn(&Vec<Points>) + 'static,
) -> impl IntoElement {
    h_resizable()
        .handle_style(|s| {
            s.w(px(1.0))
                .bg(move || theme().border)
                .group_hover(|s| s.w(px(2.0)).bg(border_strong))
                .group_active(|s| s.w(px(2.0)).bg(move || theme().primary))
        })
        .handle_accessibility_label("Resize sidebar panel")
        .on_resize(on_resize)
        .child(
            resizable_panel()
                .size(left_initial)
                .size_range(left_min..left_max)
                .child(left_pane),
        )
        .child(resizable_panel().child(right_pane))
}
