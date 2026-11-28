//! Read-only code, diff, image, archive, and structured binary viewers.

use nsh_base::{DiffLayout, diff_view, editor, theme};
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

use crate::domain::{ArchivePreviewMeta, StructuredPreviewMeta, format_bytes};
use crate::ui::theme::{font_mono_family, radius_control, radius_inset};

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
                .font_size(px(12.0))
                .line_height(px(19.0))
                .accessibility_label(accessibility_label.into())
                .flex_1(),
        )
}

/// Read-only hex dump viewer without redundant line numbers (hex rows already carry byte offsets).
pub fn ui_hex_viewer(
    content: impl IntoReactive<Option<SharedString>>,
    accessibility_label: impl Into<SharedString>,
) -> impl IntoElement {
    let content = content.into_reactive();
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
                .read_only(true)
                .line_numbers(false)
                .soft_wrap(false)
                .folding(false)
                .font_family(font_mono_family())
                .font_size(px(12.0))
                .line_height(px(18.5))
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
                .font_size(px(12.0))
                .line_height(px(19.0))
                .flex_1(),
        )
}

fn stage_display_dimensions(natural_size: Option<(u32, u32)>, fit_to_stage: bool) -> (f32, f32) {
    let Some((w, h)) = natural_size else {
        return (360.0, 240.0);
    };
    let w = (w.max(1) as f32).min(4096.0);
    let h = (h.max(1) as f32).min(4096.0);
    if !fit_to_stage {
        return (w, h);
    }
    const MAX_FIT_W: f32 = 560.0;
    const MAX_FIT_H: f32 = 360.0;
    if w <= MAX_FIT_W && h <= MAX_FIT_H {
        return (w, h);
    }
    let scale = (MAX_FIT_W / w).min(MAX_FIT_H / h);
    ((w * scale).max(1.0), (h * scale).max(1.0))
}

/// Framed image / SVG preview stage using semantic surface tokens.
pub fn ui_image_stage(
    data_uri: Option<SharedString>,
    svg_markup: Option<SharedString>,
    natural_size: Option<(u32, u32)>,
    fit_to_stage: bool,
    accessibility_label: impl Into<SharedString>,
) -> impl IntoElement {
    let label = accessibility_label.into();
    let (disp_w, disp_h) = stage_display_dimensions(natural_size, fit_to_stage);
    let media_el: AnyElement = if let Some(markup) = svg_markup {
        svg(ImageSrc::svg_markup(markup.as_str()))
            .w(px(disp_w))
            .h(px(disp_h))
            .role(AccessibilityRole::Image)
            .label(label)
            .into_element()
    } else if let Some(uri) = data_uri {
        image(uri)
            .object_fit(ResizeMode::Contain)
            .w(px(disp_w))
            .h(px(disp_h))
            .role(AccessibilityRole::Image)
            .label(label)
            .into_element()
    } else {
        view().into_element()
    };

    scroll()
        .flex_1()
        .min_h(px(0.0))
        .rounded(radius_control)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().muted)
        .child(
            view()
                .flex_1()
                .min_h(px(240.0))
                .p_6()
                .items_center()
                .justify_center()
                .child(
                    view()
                        .rounded(radius_inset)
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().card)
                        .p_4()
                        .items_center()
                        .justify_center()
                        .child(media_el),
                ),
        )
}

/// Structured binary inspector view displaying summary, key/value fields, and sections.
pub fn ui_structured_viewer(meta: &StructuredPreviewMeta) -> impl IntoElement {
    let summary = meta.summary.clone();
    let fields = meta.fields.clone();
    let sections = meta.sections.clone();

    scroll()
        .flex_1()
        .min_h(px(0.0))
        .rounded(radius_control)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().muted)
        .child(
            view()
                .flex_col()
                .gap_3()
                .p_3()
                .child(
                    view()
                        .rounded(radius_inset)
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().card)
                        .px_3()
                        .py_2p5()
                        .child(
                            text(summary)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        ),
                )
                .when(!fields.is_empty(), |col| {
                    let row_count = fields.len();
                    col.child(
                        view()
                            .rounded(radius_inset)
                            .border_1()
                            .border_color(move || theme().border)
                            .bg(move || theme().card)
                            .flex_col()
                            .children(fields.into_iter().enumerate().map(move |(idx, entry)| {
                                view()
                                    .flex_row()
                                    .items_start()
                                    .gap_3()
                                    .px_3()
                                    .py_1p5()
                                    .when(idx + 1 < row_count, |row| {
                                        row.border_b_1().border_color(move || theme().border)
                                    })
                                    .child(
                                        text(entry.label)
                                            .w(px(150.0))
                                            .flex_shrink_0()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(move || theme().muted_foreground),
                                    )
                                    .child(
                                        text(entry.value)
                                            .flex_1()
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .text_color(move || theme().foreground),
                                    )
                            })),
                    )
                })
                .children(sections.into_iter().filter_map(|sec| {
                    if sec.items.is_empty() {
                        return None;
                    }
                    let joined = sec.items.join("\n");
                    Some(
                        view()
                            .flex_col()
                            .gap_1()
                            .child(
                                text(sec.title)
                                    .role(AccessibilityRole::Header)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(
                                view()
                                    .rounded(radius_inset)
                                    .border_1()
                                    .border_color(move || theme().border)
                                    .bg(move || theme().card)
                                    .px_3()
                                    .py_2()
                                    .child(
                                        text(joined)
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .text_color(move || theme().foreground),
                                    ),
                            ),
                    )
                })),
        )
}

/// Tabular archive contents viewer showing permissions, uncompressed size, and entry path.
pub fn ui_archive_viewer(
    meta: &ArchivePreviewMeta,
    col_mode: impl IntoReactive<SharedString>,
    col_size: impl IntoReactive<SharedString>,
    col_path: impl IntoReactive<SharedString>,
) -> impl IntoElement {
    let entries = meta.entries.clone();
    let entry_count = entries.len();
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .rounded(radius_control)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .overflow_hidden()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_1p5()
                .bg(move || theme().muted)
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    text(col_mode)
                        .w(px(92.0))
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(col_size)
                        .w(px(76.0))
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(col_path)
                        .flex_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .child(
                    view()
                        .flex_col()
                        .children(entries.into_iter().enumerate().map(move |(idx, entry)| {
                            let size_text = format_bytes(entry.size);
                            view()
                                .flex_row()
                                .items_start()
                                .gap_3()
                                .px_3()
                                .py_1()
                                .when(idx + 1 < entry_count, |row| {
                                    row.border_b_1().border_color(move || theme().border)
                                })
                                .child(
                                    text(entry.mode)
                                        .w(px(92.0))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(size_text)
                                        .w(px(76.0))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(entry.path)
                                        .flex_1()
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .text_color(move || theme().foreground),
                                )
                        })),
                ),
        )
}
