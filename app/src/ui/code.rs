//! Read-only code and diff viewers wrapping `nsh_base::editor` and `nsh_base::diff_view`.

use nsh_base::{DiffLayout, diff_view, editor, theme};
use nsh_core::prelude::*;

use crate::ui::theme::{font_mono_family, radius_control};

/// Read-only syntax-highlighted code viewer (used for Dockerfile & text file preview).
pub fn ui_code_viewer(
    content: impl IntoReactive<Option<SharedString>>,
    language: impl IntoReactive<Option<SharedString>>,
    soft_wrap: impl IntoReactive<bool>,
    accessibility_label: impl Into<SharedString>,
) -> impl IntoElement {
    let content = content.into_reactive();
    let language = language.into_reactive();
    let soft_wrap = soft_wrap.into_reactive();
    view()
        .flex_1()
        .min_h(px(0.0))
        .rounded(radius_control)
        .bg(move || theme().muted)
        .border_1()
        .border_color(move || theme().border)
        .overflow_hidden()
        .child(
            editor()
                .value(move || content.get())
                .language(move || language.get())
                .read_only(true)
                .line_numbers(true)
                .soft_wrap(move || soft_wrap.get())
                .folding(true)
                .font_family(font_mono_family())
                .font_size(px(12.5))
                .line_height(px(19.0))
                .accessibility_label(accessibility_label.into())
                .flex_1(),
        )
}

/// Read-only syntax-highlighted patch/text diff viewer built over `nsh_base::diff_view`.
pub fn ui_diff_viewer(
    old_value: impl IntoReactive<Option<SharedString>>,
    new_value: impl IntoReactive<Option<SharedString>>,
    language: impl IntoReactive<Option<SharedString>>,
    split: bool,
) -> impl IntoElement {
    let old_value = old_value.into_reactive();
    let new_value = new_value.into_reactive();
    let language = language.into_reactive();
    view()
        .flex_1()
        .min_h(px(0.0))
        .rounded(radius_control)
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .overflow_hidden()
        .child(
            diff_view()
                .old_value(move || old_value.get())
                .new_value(move || new_value.get())
                .language(move || language.get())
                .layout(if split {
                    DiffLayout::Split
                } else {
                    DiffLayout::Unified
                })
                .font_family(font_mono_family())
                .font_size(px(12.5))
                .line_height(px(19.0))
                .flex_1(),
        )
}
