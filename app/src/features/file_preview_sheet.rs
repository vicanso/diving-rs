//! Right-edge File Preview Sheet (`CodeViewer` for UTF-8 text, hex dump for binary files).

use std::time::Duration;

use nsh_core::prelude::*;

use crate::domain::{AppState, FilePreviewContent, format_bytes};
use crate::i18n::{t, tf};
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, UiSize, theme, ui_badge, ui_banner, ui_button,
    ui_code_viewer, ui_empty_state, ui_separator, ui_sheet, ui_skeleton,
};

/// Mounts the right-sliding inspection drawer bound to `state.preview_sheet_open`.
pub fn file_preview_sheet(state: AppState) -> impl IntoElement {
    let sheet_state = state.clone();
    let title = move || -> SharedString {
        state
            .preview_target
            .get()
            .map(|target| SharedString::from(target.path))
            .unwrap_or_else(|| SharedString::from("File Preview"))
    };

    ui_sheet(state.preview_sheet_open, title, px(620.0), move || {
        sheet_body(sheet_state.clone()).into_element()
    })
}

fn sheet_body(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let copied = signal(false);
    let header_state = state.clone();
    let content_state = state.clone();

    view()
        .flex_1()
        .flex_col()
        .min_h(px(0.0))
        .gap_3()
        .child(dynamic(move || {
            let Some(target) = header_state.preview_target.get() else {
                return view().into_element();
            };
            let copy_state = header_state.clone();
            let copy_action_state = header_state.clone();
            let save_state = header_state.clone();
            let layer_label = format!("Layer #{}", target.layer_index + 1);
            let size_label = format_bytes(target.size);
            let mode_label = if target.mode.is_empty() {
                "—".to_owned()
            } else {
                target.mode.clone()
            };
            let uid_label = format!("{}:{}", target.uid, target.gid);

            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(ui_badge(layer_label, BadgeTone::Neutral))
                        .child(ui_badge(size_label, BadgeTone::Neutral))
                        .child(ui_badge(mode_label, BadgeTone::Neutral))
                        .child(ui_badge(uid_label, BadgeTone::Neutral)),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(dynamic(move || {
                            let btn_state = copy_state.clone();
                            let is_copied = copied.get();
                            ui_button(
                                move || {
                                    SharedString::from(if copied.get() {
                                        t(lang.get(), "common.copied")
                                    } else {
                                        t(lang.get(), "common.copy")
                                    })
                                },
                                ButtonVariant::Outline,
                                UiSize::Xs,
                                Some(if is_copied {
                                    IconName::Check
                                } else {
                                    IconName::Copy
                                }),
                                move || {
                                    !matches!(
                                        btn_state.preview_content.get(),
                                        Some(
                                            FilePreviewContent::Text { .. }
                                                | FilePreviewContent::BinaryHex { .. }
                                        )
                                    )
                                },
                                false,
                                move || {
                                    let payload = match copy_action_state
                                        .preview_content
                                        .get_untracked()
                                    {
                                        Some(FilePreviewContent::Text { text, .. }) => Some(text),
                                        Some(FilePreviewContent::BinaryHex {
                                            hex_dump, ..
                                        }) => Some(hex_dump),
                                        _ => None,
                                    };
                                    if let Some(body) = payload {
                                        set_clipboard(body);
                                        copied.set(true);
                                        set_timeout(Duration::from_millis(1500), move || {
                                            copied.set(false);
                                        });
                                    }
                                },
                            )
                        }))
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "common.save_as")),
                            ButtonVariant::Outline,
                            UiSize::Xs,
                            Some(IconName::Download),
                            false,
                            false,
                            move || save_state.save_preview_file(),
                        )),
                )
                .into_element()
        }))
        .child(ui_separator())
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .flex_col()
                .child(dynamic(move || {
                    if content_state.preview_loading.get() {
                        return view()
                            .flex_col()
                            .gap_3()
                            .p_2()
                            .child(
                                text(move || t(lang.get(), "sheet.loading"))
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(ui_skeleton(px(360.0), px(14.0)))
                            .child(ui_skeleton(px(440.0), px(14.0)))
                            .child(ui_skeleton(px(300.0), px(14.0)))
                            .into_element();
                    }
                    if let Some(err) = content_state.preview_error.get() {
                        return ui_banner(BannerTone::Danger, SharedString::from(err), None)
                            .into_element();
                    }
                    match content_state.preview_content.get() {
                        Some(FilePreviewContent::Empty) => ui_empty_state(
                            IconName::File,
                            SharedString::from(t(lang.get(), "sheet.empty")),
                            "",
                            None,
                        )
                        .into_element(),
                        Some(FilePreviewContent::Text {
                            text,
                            language,
                            shown_bytes,
                            total_size,
                            truncated,
                        }) => view()
                            .flex_1()
                            .min_h(px(0.0))
                            .flex_col()
                            .gap_2()
                            .when(truncated, |col| {
                                col.child(ui_banner(
                                    BannerTone::Info,
                                    tf(
                                        lang.get_untracked(),
                                        "sheet.truncated",
                                        &[&format_bytes(shown_bytes), &format_bytes(total_size)],
                                    ),
                                    None,
                                ))
                            })
                            .child(ui_code_viewer(
                                Some(SharedString::from(text)),
                                language.map(SharedString::from),
                                true,
                                "File text content viewer",
                            ))
                            .into_element(),
                        Some(FilePreviewContent::BinaryHex {
                            hex_dump,
                            shown_bytes,
                            total_size,
                            ..
                        }) => view()
                            .flex_1()
                            .min_h(px(0.0))
                            .flex_col()
                            .gap_2()
                            .child(ui_banner(
                                BannerTone::Info,
                                tf(
                                    lang.get_untracked(),
                                    "sheet.binary",
                                    &[&format_bytes(shown_bytes), &format_bytes(total_size)],
                                ),
                                None,
                            ))
                            .child(ui_code_viewer(
                                Some(SharedString::from(hex_dump)),
                                None::<SharedString>,
                                false,
                                "Binary file hex dump viewer",
                            ))
                            .into_element(),
                        None => view().into_element(),
                    }
                })),
        )
}
