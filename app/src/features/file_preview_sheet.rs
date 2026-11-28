//! Right-edge File Preview Sheet with rich image/SVG/archive/ELF/JSON viewers and hex dump toggle.

use std::time::Duration;

use diving_core::PREVIEW_RICH_LIMIT;
use nsh_core::prelude::*;

use crate::domain::{
    AppState, FileKind, FilePreviewContent, PreviewField, StructuredPreviewMeta,
    classify_file_kind, format_bytes,
};
use crate::i18n::{format_user_error, t, tf};
use crate::status_notice_banner;
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, SegmentOption, UiSize, file_kind_icon, theme,
    ui_archive_viewer, ui_badge, ui_banner, ui_button, ui_code_viewer, ui_empty_state,
    ui_hex_viewer, ui_image_stage, ui_segmented, ui_separator, ui_sheet, ui_skeleton,
    ui_structured_viewer,
};

fn save_as_disabled(state: &AppState) -> bool {
    state.preview_loading.get()
        || state.preview_error.get().is_some()
        || state.preview_content.get().is_none()
        || state
            .preview_target
            .get()
            .is_some_and(|target| target.mode.starts_with('l'))
}

fn copyable_payload(content: Option<&FilePreviewContent>, secondary_mode: bool) -> Option<String> {
    match content? {
        FilePreviewContent::Text {
            text,
            formatted_text,
            ..
        } => {
            if !secondary_mode && let Some(formatted) = formatted_text {
                Some(formatted.clone())
            } else {
                Some(text.clone())
            }
        }
        FilePreviewContent::Svg { source, .. } => Some(source.clone()),
        FilePreviewContent::Image { hex_dump, .. } => secondary_mode.then(|| hex_dump.clone()),
        FilePreviewContent::Archive {
            report_text,
            hex_dump,
            ..
        }
        | FilePreviewContent::Structured {
            report_text,
            hex_dump,
            ..
        }
        | FilePreviewContent::Media {
            report_text,
            hex_dump,
            ..
        } => Some(if secondary_mode {
            hex_dump.clone()
        } else {
            report_text.clone()
        }),
        FilePreviewContent::BinaryHex { hex_dump, .. } => Some(hex_dump.clone()),
        FilePreviewContent::Empty => None,
    }
}

fn mode_labels(content: &FilePreviewContent) -> Option<(&'static str, &'static str)> {
    match content {
        FilePreviewContent::Image { .. } | FilePreviewContent::Media { .. } => {
            Some(("sheet.mode_preview", "sheet.mode_hex"))
        }
        FilePreviewContent::Svg { .. } => Some(("sheet.mode_preview", "sheet.mode_source")),
        FilePreviewContent::Archive { .. } => Some(("sheet.mode_contents", "sheet.mode_hex")),
        FilePreviewContent::Structured { .. } => Some(("sheet.mode_inspector", "sheet.mode_hex")),
        FilePreviewContent::Text {
            formatted_text: Some(_),
            ..
        } => Some(("sheet.mode_formatted", "sheet.mode_raw")),
        _ => None,
    }
}

fn preview_meta_badges(content: Option<&FilePreviewContent>) -> Vec<(String, BadgeTone)> {
    let mut badges = Vec::new();
    match content {
        Some(FilePreviewContent::Image { meta, .. } | FilePreviewContent::Svg { meta, .. }) => {
            badges.push((meta.format.clone(), BadgeTone::Info));
            if let (Some(w), Some(h)) = (meta.width, meta.height) {
                badges.push((format!("{w} × {h}"), BadgeTone::Neutral));
            }
            if let Some(detail) = &meta.detail {
                badges.push((detail.clone(), BadgeTone::Neutral));
            }
        }
        Some(FilePreviewContent::Media { meta, .. }) => {
            badges.push((meta.format.clone(), BadgeTone::Info));
            if let Some(detail) = &meta.detail {
                badges.push((detail.clone(), BadgeTone::Neutral));
            }
        }
        Some(FilePreviewContent::Archive { meta, .. }) => {
            badges.push((meta.format.clone(), BadgeTone::Info));
        }
        Some(FilePreviewContent::Structured { meta, .. }) => {
            badges.push((meta.format.clone(), BadgeTone::Info));
        }
        Some(FilePreviewContent::Text {
            language,
            compression,
            ..
        }) => {
            if let Some(lang) = language {
                badges.push(((*lang).to_owned(), BadgeTone::Neutral));
            }
            if let Some(codec) = compression {
                badges.push(((*codec).to_owned(), BadgeTone::Info));
            }
        }
        _ => {}
    }
    badges
}

fn hex_view_panel(
    state: &AppState,
    hex_dump: String,
    shown_bytes: u64,
    total_size: u64,
) -> AnyElement {
    let lang = state.lang;
    let shown = format_bytes(shown_bytes);
    let total = format_bytes(total_size);
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .gap_2()
        .child(ui_banner(
            BannerTone::Info,
            move || SharedString::from(tf(lang.get(), "sheet.binary", &[&shown, &total])),
            None,
        ))
        .child(ui_hex_viewer(
            Some(SharedString::from(hex_dump)),
            "Binary file hex dump viewer",
        ))
        .into_element()
}

/// Mounts the right-sliding inspection drawer bound to `state.preview_sheet_open`.
pub fn file_preview_sheet(state: AppState) -> impl IntoElement {
    let sheet_state = state.clone();
    let lang = state.lang;
    let title = move || -> SharedString {
        state
            .preview_target
            .get()
            .map(|target| SharedString::from(target.path))
            .unwrap_or_else(|| SharedString::from(t(lang.get(), "sheet.title_default")))
    };

    ui_sheet(state.preview_sheet_open, title, px(700.0), move || {
        sheet_body(sheet_state.clone()).into_element()
    })
}

fn sheet_body(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let copied = signal(false);
    let copy_seq = signal(0_u64);
    let secondary_mode = signal(false);
    let image_fit = signal(true);

    let header_state = state.clone();
    let toolbar_state = state.clone();
    let notice_state = state.clone();
    let content_state = state.clone();
    let target_reset_state = state.clone();

    effect(move || {
        let _ = target_reset_state.preview_target.get();
        copy_seq.set(copy_seq.get_untracked().wrapping_add(1));
        copied.set(false);
        secondary_mode.set(false);
        image_fit.set(true);
    });

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
            let layer_num = (target.layer_index + 1).to_string();
            let size_label = format_bytes(target.size);
            let is_symlink = target.mode.starts_with('l');
            let mode_label = if target.mode.is_empty() {
                "—".to_owned()
            } else {
                target.mode.clone()
            };
            let uid_label = format!("{}:{}", target.uid, target.gid);
            let extra_badges = preview_meta_badges(header_state.preview_content.get().as_ref());

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
                        .child(ui_badge(
                            move || {
                                SharedString::from(tf(
                                    lang.get(),
                                    "common.layer_number",
                                    &[&layer_num],
                                ))
                            },
                            BadgeTone::Neutral,
                        ))
                        .when(is_symlink, |row| {
                            row.child(ui_badge(
                                move || SharedString::from(t(lang.get(), "sheet.symlink")),
                                BadgeTone::Info,
                            ))
                        })
                        .child(ui_badge(size_label, BadgeTone::Neutral))
                        .child(ui_badge(mode_label, BadgeTone::Neutral))
                        .child(ui_badge(uid_label, BadgeTone::Neutral))
                        .children(
                            extra_badges
                                .into_iter()
                                .map(|(label, tone)| ui_badge(label, tone)),
                        ),
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
                                    copyable_payload(
                                        btn_state.preview_content.get().as_ref(),
                                        secondary_mode.get(),
                                    )
                                    .is_none()
                                },
                                false,
                                move || {
                                    let payload = copyable_payload(
                                        copy_action_state.preview_content.get_untracked().as_ref(),
                                        secondary_mode.get_untracked(),
                                    );
                                    if let Some(body) = payload {
                                        drop(set_clipboard(body));
                                        let next_seq = copy_seq.get_untracked().wrapping_add(1);
                                        copy_seq.set(next_seq);
                                        copied.set(true);
                                        set_timeout(Duration::from_millis(1500), move || {
                                            if copy_seq.try_get() == Some(next_seq) {
                                                copied.try_set(false);
                                            }
                                        });
                                    }
                                },
                            )
                        }))
                        .child({
                            let save_disabled_state = save_state.clone();
                            ui_button(
                                move || SharedString::from(t(lang.get(), "common.save_as")),
                                ButtonVariant::Outline,
                                UiSize::Xs,
                                Some(IconName::Download),
                                move || save_as_disabled(&save_disabled_state),
                                false,
                                move || save_state.save_preview_file(),
                            )
                        }),
                )
                .into_element()
        }))
        .child(dynamic(move || {
            let content = toolbar_state.preview_content.get();
            let Some(preview_ref) = content.as_ref() else {
                return view().into_element();
            };
            let Some((primary_key, secondary_key)) = mode_labels(preview_ref) else {
                return view().into_element();
            };
            let show_fit = match preview_ref {
                FilePreviewContent::Image { meta, data_uri, .. } => {
                    data_uri.is_some() && meta.width.is_some() && meta.height.is_some()
                }
                FilePreviewContent::Svg { meta, .. } => {
                    meta.width.is_some() && meta.height.is_some()
                }
                _ => false,
            };

            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(ui_segmented(
                    vec![
                        SegmentOption::new(false, move || {
                            SharedString::from(t(lang.get(), primary_key))
                        }),
                        SegmentOption::new(true, move || {
                            SharedString::from(t(lang.get(), secondary_key))
                        }),
                    ],
                    secondary_mode,
                    UiSize::Xs,
                    "Switch preview display mode",
                    move |mode| secondary_mode.set(mode),
                ))
                .when(show_fit, |row| {
                    row.child(when(
                        move || !secondary_mode.get(),
                        move || {
                            ui_segmented(
                                vec![
                                    SegmentOption::new(true, move || {
                                        SharedString::from(t(lang.get(), "sheet.fit_window"))
                                    }),
                                    SegmentOption::new(false, move || {
                                        SharedString::from(t(lang.get(), "sheet.actual_size"))
                                    }),
                                ],
                                image_fit,
                                UiSize::Xs,
                                "Toggle image fit or actual size",
                                move |fit| image_fit.set(fit),
                            )
                        },
                    ))
                })
                .into_element()
        }))
        .child(ui_separator())
        .child(when(
            move || notice_state.status_notice.get().is_some(),
            move || status_notice_banner(state.clone()),
        ))
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
                        return ui_banner(
                            BannerTone::Danger,
                            move || {
                                let msg = if err == "error.preview_cancelled" {
                                    t(lang.get(), "error.preview_cancelled").to_owned()
                                } else {
                                    format_user_error(lang.get(), &err)
                                };
                                SharedString::from(msg)
                            },
                            None,
                        )
                        .into_element();
                    }
                    let target_info = content_state.preview_target.get();
                    let is_symlink = target_info
                        .as_ref()
                        .is_some_and(|target| target.mode.starts_with('l'));
                    let file_kind = target_info.map_or(FileKind::Generic, |target| {
                        classify_file_kind(&target.path, &target.mode, is_symlink)
                    });
                    let is_secondary = secondary_mode.get();

                    match content_state.preview_content.get() {
                        Some(FilePreviewContent::Empty) => ui_empty_state(
                            file_kind_icon(file_kind),
                            move || SharedString::from(t(lang.get(), "sheet.empty")),
                            "",
                            None,
                        )
                        .into_element(),
                        Some(FilePreviewContent::Text { text, .. }) if is_symlink => view()
                            .flex_1()
                            .min_h(px(0.0))
                            .items_center()
                            .justify_center()
                            .child(ui_empty_state(
                                IconName::Link,
                                move || SharedString::from(t(lang.get(), "sheet.symlink")),
                                SharedString::from(text),
                                None,
                            ))
                            .into_element(),
                        Some(FilePreviewContent::Text {
                            text,
                            formatted_text,
                            language,
                            compression,
                            shown_bytes,
                            total_size,
                            truncated,
                        }) => {
                            let displayed = if !is_secondary && let Some(pretty) = formatted_text {
                                pretty
                            } else {
                                text
                            };
                            let shown = format_bytes(shown_bytes);
                            let total = format_bytes(total_size);
                            view()
                                .flex_1()
                                .min_h(px(0.0))
                                .flex_col()
                                .gap_2()
                                .when_some(compression, |col, codec| {
                                    let shown_str = shown.clone();
                                    col.child(ui_banner(
                                        BannerTone::Info,
                                        move || {
                                            SharedString::from(tf(
                                                lang.get(),
                                                "sheet.decompressed",
                                                &[codec, &shown_str],
                                            ))
                                        },
                                        None,
                                    ))
                                })
                                .when(truncated && compression.is_none(), |col| {
                                    let shown_str = shown.clone();
                                    let total_str = total.clone();
                                    col.child(ui_banner(
                                        BannerTone::Info,
                                        move || {
                                            SharedString::from(tf(
                                                lang.get(),
                                                "sheet.truncated",
                                                &[&shown_str, &total_str],
                                            ))
                                        },
                                        None,
                                    ))
                                })
                                .child(ui_code_viewer(
                                    Some(SharedString::from(displayed)),
                                    language.map(SharedString::from),
                                    true,
                                    "File text content viewer",
                                ))
                                .into_element()
                        }
                        Some(FilePreviewContent::Svg { meta, source, .. }) => {
                            if is_secondary {
                                ui_code_viewer(
                                    Some(SharedString::from(source)),
                                    Some(SharedString::from("xml")),
                                    true,
                                    "SVG XML source viewer",
                                )
                                .into_element()
                            } else {
                                let dim = meta.width.zip(meta.height);
                                ui_image_stage(
                                    None,
                                    Some(SharedString::from(source)),
                                    dim,
                                    image_fit.get(),
                                    "SVG vector preview",
                                )
                                .into_element()
                            }
                        }
                        Some(FilePreviewContent::Image {
                            meta,
                            data_uri,
                            hex_dump,
                            shown_bytes,
                            total_size,
                            truncated,
                        }) => {
                            if is_secondary {
                                return hex_view_panel(
                                    &content_state,
                                    hex_dump,
                                    shown_bytes,
                                    total_size,
                                );
                            }
                            if let Some(uri) = data_uri
                                && !truncated
                            {
                                let dim = meta.width.zip(meta.height);
                                ui_image_stage(
                                    Some(SharedString::from(uri)),
                                    None,
                                    dim,
                                    image_fit.get(),
                                    "Raster image preview",
                                )
                                .into_element()
                            } else {
                                let cap_label = format_bytes(PREVIEW_RICH_LIMIT);
                                view()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .flex_col()
                                    .gap_2()
                                    .child(ui_banner(
                                        BannerTone::Warning,
                                        move || {
                                            SharedString::from(tf(
                                                lang.get(),
                                                "sheet.image_too_large",
                                                &[&cap_label],
                                            ))
                                        },
                                        None,
                                    ))
                                    .child(ui_hex_viewer(
                                        Some(SharedString::from(hex_dump)),
                                        "Large image hex dump fallback viewer",
                                    ))
                                    .into_element()
                            }
                        }
                        Some(FilePreviewContent::Archive {
                            meta,
                            hex_dump,
                            shown_bytes,
                            total_size,
                            ..
                        }) => {
                            if is_secondary {
                                return hex_view_panel(
                                    &content_state,
                                    hex_dump,
                                    shown_bytes,
                                    total_size,
                                );
                            }
                            let fmt = meta.format.clone();
                            let count = meta.total_entries.to_string();
                            let uncompressed = format_bytes(meta.total_uncompressed_size);
                            let shown_entries = meta.entries.len().to_string();
                            let truncated_entries = meta.truncated_entries;
                            let count_copy = count.clone();
                            view()
                                .flex_1()
                                .min_h(px(0.0))
                                .flex_col()
                                .gap_2()
                                .child(ui_banner(
                                    BannerTone::Info,
                                    move || {
                                        let mut msg = tf(
                                            lang.get(),
                                            "sheet.archive_summary",
                                            &[&fmt, &count, &uncompressed],
                                        );
                                        if truncated_entries {
                                            msg.push_str(" · ");
                                            msg.push_str(&tf(
                                                lang.get(),
                                                "sheet.archive_truncated",
                                                &[&shown_entries, &count_copy],
                                            ));
                                        }
                                        SharedString::from(msg)
                                    },
                                    None,
                                ))
                                .child(ui_archive_viewer(
                                    &meta,
                                    move || {
                                        SharedString::from(t(lang.get(), "sheet.archive_col_mode"))
                                    },
                                    move || {
                                        SharedString::from(t(lang.get(), "sheet.archive_col_size"))
                                    },
                                    move || {
                                        SharedString::from(t(lang.get(), "sheet.archive_col_path"))
                                    },
                                ))
                                .into_element()
                        }
                        Some(FilePreviewContent::Structured {
                            meta,
                            hex_dump,
                            shown_bytes,
                            total_size,
                            ..
                        }) => {
                            if is_secondary {
                                hex_view_panel(&content_state, hex_dump, shown_bytes, total_size)
                            } else {
                                ui_structured_viewer(&meta).into_element()
                            }
                        }
                        Some(FilePreviewContent::Media {
                            meta,
                            hex_dump,
                            shown_bytes,
                            total_size,
                            ..
                        }) => {
                            if is_secondary {
                                return hex_view_panel(
                                    &content_state,
                                    hex_dump,
                                    shown_bytes,
                                    total_size,
                                );
                            }
                            let mut fields = vec![
                                PreviewField {
                                    label: "Format".to_owned(),
                                    value: meta.format,
                                },
                                PreviewField {
                                    label: "MIME".to_owned(),
                                    value: meta.mime,
                                },
                                PreviewField {
                                    label: "Size".to_owned(),
                                    value: format_bytes(total_size),
                                },
                            ];
                            if let Some(detail) = meta.detail {
                                fields.push(PreviewField {
                                    label: "Stream".to_owned(),
                                    value: detail,
                                });
                            }
                            let media_summary = StructuredPreviewMeta {
                                format: fields[0].value.clone(),
                                summary: format!("{} ({})", fields[0].value, fields[2].value),
                                fields,
                                sections: Vec::new(),
                            };
                            view()
                                .flex_1()
                                .min_h(px(0.0))
                                .flex_col()
                                .gap_2()
                                .child(ui_banner(
                                    BannerTone::Info,
                                    move || SharedString::from(t(lang.get(), "sheet.media_hint")),
                                    None,
                                ))
                                .child(ui_structured_viewer(&media_summary))
                                .into_element()
                        }
                        Some(FilePreviewContent::BinaryHex {
                            hex_dump,
                            shown_bytes,
                            total_size,
                            ..
                        }) => hex_view_panel(&content_state, hex_dump, shown_bytes, total_size),
                        None => view().into_element(),
                    }
                })),
        )
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use diving_core::Analyzer;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use nsh_core::clipboard::{ClipboardBackend, MemoryClipboard};
    use nsh_engine::{Clock, EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, Screen, windowed_root};
    use tar::{Builder as TarBuilder, Header as TarHeader};

    use super::*;
    use crate::domain::{DivingService, PreviewTarget, classify_file_preview};
    use crate::i18n::UiLang;
    use crate::ui::app_tokens;

    fn test_state() -> AppState {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        state.preview_target.set(Some(PreviewTarget {
            image_ref: "demo:v1".to_owned(),
            layer_index: 0,
            layer_digest: "sha256:abc".to_owned(),
            media_type: String::new(),
            path: "etc/app.conf".to_owned(),
            size: 16,
            mode: "-rw-r--r--".to_owned(),
            uid: 0,
            gid: 0,
        }));
        state.preview_content.set(Some(FilePreviewContent::Text {
            text: "port = 8080\n".to_owned(),
            formatted_text: None,
            language: Some("ini"),
            compression: None,
            shown_bytes: 12,
            total_size: 12,
            truncated: false,
        }));
        state.preview_sheet_open.set(true);
        state
    }

    #[component]
    fn SheetTimerHarness() -> impl IntoElement {
        let state = test_state();
        provide_context(state.clone());
        view()
            .flex_1()
            .bg(move || theme().background)
            .child(file_preview_sheet(state))
    }

    #[test]
    fn copy_timer_does_not_panic_when_sheet_closes_before_1500ms() {
        let clipboard = Arc::new(MemoryClipboard::new(None));
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(960.0), Points(640.0)),
            clock: Clock::manual(SystemTime::UNIX_EPOCH),
            clipboard: Some(clipboard.clone()),
            ..EngineConfig::new(windowed_root::<SheetTimerHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start sheet harness");
        app.wait_for_within("Copy", Duration::from_secs(2))
            .expect("Copy button visible");
        app.press("Copy").expect("click Copy");
        assert_eq!(clipboard.text().as_deref(), Some("port = 8080\n"));
        app.wait_for_within("Copied", Duration::from_secs(1))
            .expect("Copied feedback shown");

        app.press("Close sheet").expect("close sheet before 1.5s");
        app.wait_for_gone("Copy", Duration::from_secs(2))
            .expect("sheet closed");
        app.fast_forward(Duration::from_millis(1600))
            .expect("1.5s copy reset timer after sheet close must not panic");
    }

    #[component]
    fn SheetNoticeBinarySymlinkHarness() -> impl IntoElement {
        let state = test_state();
        provide_context(state.clone());
        state.show_status_notice("Saved to /tmp/app.conf.".to_owned());
        state.preview_target.set(Some(PreviewTarget {
            image_ref: "demo:v1".to_owned(),
            layer_index: 1,
            layer_digest: "sha256:def".to_owned(),
            media_type: String::new(),
            path: "bin/demo.bin".to_owned(),
            size: 1024,
            mode: "-rwxr-xr-x".to_owned(),
            uid: 0,
            gid: 0,
        }));
        state
            .preview_content
            .set(Some(FilePreviewContent::BinaryHex {
                hex_dump: "00000000  7f 45 4c 46  |.ELF|".to_owned(),
                shown_bytes: 4,
                total_size: 1024,
                truncated: true,
            }));

        let s_sym = state.clone();
        view()
            .flex_1()
            .bg(move || theme().background)
            .child(
                ui_button(
                    "Switch to symlink",
                    ButtonVariant::Default,
                    UiSize::Sm,
                    None,
                    false,
                    false,
                    move || {
                        s_sym.preview_target.set(Some(PreviewTarget {
                            image_ref: "demo:v1".to_owned(),
                            layer_index: 1,
                            layer_digest: "sha256:def".to_owned(),
                            media_type: String::new(),
                            path: "usr/bin/sh".to_owned(),
                            size: 0,
                            mode: "lrwxrwxrwx".to_owned(),
                            uid: 0,
                            gid: 0,
                        }));
                        s_sym.preview_content.set(Some(FilePreviewContent::Text {
                            text: "Symbolic link → busybox".to_owned(),
                            formatted_text: None,
                            language: None,
                            compression: None,
                            shown_bytes: 25,
                            total_size: 25,
                            truncated: false,
                        }));
                        s_sym.preview_sheet_open.set(true);
                    },
                )
                .into_element(),
            )
            .child(file_preview_sheet(state))
    }

    #[test]
    fn sheet_shows_status_notice_binary_sizes_and_symlink_info() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(
                windowed_root::<SheetNoticeBinarySymlinkHarness>().tokens(app_tokens()),
            )
        };
        let mut app = Headless::start(config).expect("start sheet features harness");
        app.wait_for_within("Saved to /tmp/app.conf.", Duration::from_secs(2))
            .expect("status notice visible inside open sheet");
        app.wait_for_within(
            "Binary file — showing first 4 B of 1.0 KiB as hex dump.",
            Duration::from_secs(2),
        )
        .expect("formatted byte placeholders in binary preview banner");
        assert!(
            !app.screen()
                .accessibility_state("role:button[Save as…]")
                .expect("save button enabled for binary content")
                .disabled
        );

        app.press("Close sheet").expect("close sheet before switch");
        app.wait_for_gone("00000000  7f 45 4c 46  |.ELF|", Duration::from_secs(2))
            .expect("sheet closed");
        app.press("Switch to symlink").expect("switch to symlink");
        app.wait_for_within("Symbolic link → busybox", Duration::from_secs(2))
            .expect("symlink target displayed cleanly");
        assert!(app.shows("Symbolic link"));
        assert!(
            app.screen()
                .accessibility_state("role:button[Save as…]")
                .expect("save button disabled for symlink")
                .disabled
        );
    }

    #[component]
    fn SheetErrorI18nHarness() -> impl IntoElement {
        let state = test_state();
        provide_context(state.clone());
        state.preview_content.set(None);
        state
            .preview_error
            .set(Some("Tar fail: truncated layer".to_owned()));

        let zh_state = state.clone();
        let cancel_state = state.clone();
        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(ui_button(
                        "Switch ZH",
                        ButtonVariant::Outline,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || zh_state.set_lang(UiLang::Zh),
                    ))
                    .child(ui_button(
                        "Cancelled err",
                        ButtonVariant::Outline,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            cancel_state
                                .preview_error
                                .set(Some("error.preview_cancelled".to_owned()));
                        },
                    )),
            )
            .child(file_preview_sheet(state))
    }

    #[test]
    fn sheet_preview_error_formats_and_updates_on_lang_toggle() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<SheetErrorI18nHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start sheet error i18n harness");
        app.wait_for_within(
            "Unable to read local archive: truncated layer",
            Duration::from_secs(2),
        )
        .expect("EN formatted preview error shown");

        app.press("Switch ZH").expect("toggle ZH");
        app.wait_for_within(
            "无法读取本地镜像归档：truncated layer",
            Duration::from_secs(2),
        )
        .expect("ZH formatted preview error updated live");

        app.press("Cancelled err")
            .expect("set cancelled preview error key");
        app.wait_for_within("文件预览任务已取消", Duration::from_secs(2))
            .expect("cancelled preview error localized in ZH");
    }

    #[component]
    fn SheetRichModesHarness() -> impl IntoElement {
        let state = test_state();
        provide_context(state.clone());

        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13_u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&32_u32.to_be_bytes());
        png.extend_from_slice(&32_u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        state.preview_target.set(Some(PreviewTarget {
            image_ref: "demo:v1".to_owned(),
            layer_index: 0,
            layer_digest: "sha256:abc".to_owned(),
            media_type: String::new(),
            path: "static/assets/icon.png".to_owned(),
            size: png.len() as u64,
            mode: "-rw-r--r--".to_owned(),
            uid: 0,
            gid: 0,
        }));
        state.preview_content.set(Some(classify_file_preview(
            "static/assets/icon.png",
            &png,
            png.len() as u64,
        )));

        let s_svg = state.clone();
        let s_json = state.clone();
        let s_elf = state.clone();
        let s_gz = state.clone();
        let s_tar = state.clone();
        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(ui_button(
                        "Load SVG",
                        ButtonVariant::Outline,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            let svg_bytes =
                                br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="24" height="24"/></svg>"#;
                            s_svg.preview_target.set(Some(PreviewTarget {
                                image_ref: "demo:v1".to_owned(),
                                layer_index: 0,
                                layer_digest: "sha256:abc".to_owned(),
                                media_type: String::new(),
                                path: "static/logo.svg".to_owned(),
                                size: svg_bytes.len() as u64,
                                mode: "-rw-r--r--".to_owned(),
                                uid: 0,
                                gid: 0,
                            }));
                            s_svg.preview_content.set(Some(classify_file_preview(
                                "static/logo.svg",
                                svg_bytes,
                                svg_bytes.len() as u64,
                            )));
                        },
                    ))
                    .child(ui_button(
                        "Load JSON",
                        ButtonVariant::Outline,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            let json_bytes = br#"{"name":"app","version":"1.2.0"}"#;
                            s_json.preview_target.set(Some(PreviewTarget {
                                image_ref: "demo:v1".to_owned(),
                                layer_index: 0,
                                layer_digest: "sha256:abc".to_owned(),
                                media_type: String::new(),
                                path: "static/version.json".to_owned(),
                                size: json_bytes.len() as u64,
                                mode: "-rw-r--r--".to_owned(),
                                uid: 0,
                                gid: 0,
                            }));
                            s_json.preview_content.set(Some(classify_file_preview(
                                "static/version.json",
                                json_bytes,
                                json_bytes.len() as u64,
                            )));
                        },
                    ))
                    .child(ui_button(
                        "Load ELF",
                        ButtonVariant::Outline,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            let mut elf = [0_u8; 64];
                            elf[0..4].copy_from_slice(b"\x7fELF");
                            elf[4] = 2;
                            elf[5] = 1;
                            elf[16..18].copy_from_slice(&3_u16.to_le_bytes());
                            elf[18..20].copy_from_slice(&62_u16.to_le_bytes());
                            s_elf.preview_target.set(Some(PreviewTarget {
                                image_ref: "demo:v1".to_owned(),
                                layer_index: 0,
                                layer_digest: "sha256:abc".to_owned(),
                                media_type: String::new(),
                                path: "usr/bin/server".to_owned(),
                                size: 4096,
                                mode: "-rwxr-xr-x".to_owned(),
                                uid: 0,
                                gid: 0,
                            }));
                            s_elf.preview_content.set(Some(classify_file_preview(
                                "usr/bin/server",
                                &elf,
                                4096,
                            )));
                        },
                    ))
                    .child(ui_button(
                        "Load GZ",
                        ButtonVariant::Outline,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            let mut enc = GzEncoder::new(Vec::new(), Compression::default());
                            enc.write_all(b"export const ready = true;\n").unwrap();
                            let gz = enc.finish().unwrap();
                            s_gz.preview_target.set(Some(PreviewTarget {
                                image_ref: "demo:v1".to_owned(),
                                layer_index: 0,
                                layer_digest: "sha256:abc".to_owned(),
                                media_type: String::new(),
                                path: "static/app.js.gz".to_owned(),
                                size: gz.len() as u64,
                                mode: "-rw-r--r--".to_owned(),
                                uid: 0,
                                gid: 0,
                            }));
                            s_gz.preview_content.set(Some(classify_file_preview(
                                "static/app.js.gz",
                                &gz,
                                gz.len() as u64,
                            )));
                        },
                    ))
                    .child(ui_button(
                        "Load TAR",
                        ButtonVariant::Outline,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            let enc = GzEncoder::new(Vec::new(), Compression::default());
                            let mut builder = TarBuilder::new(enc);
                            let data = b"hello\n";
                            let mut hdr = TarHeader::new_gnu();
                            hdr.set_size(data.len() as u64);
                            hdr.set_mode(0o644);
                            hdr.set_uid(0);
                            hdr.set_gid(0);
                            hdr.set_cksum();
                            builder
                                .append_data(&mut hdr, "pkg/readme.txt", &data[..])
                                .unwrap();
                            let tar_gz = builder.into_inner().unwrap().finish().unwrap();
                            s_tar.preview_target.set(Some(PreviewTarget {
                                image_ref: "demo:v1".to_owned(),
                                layer_index: 0,
                                layer_digest: "sha256:abc".to_owned(),
                                media_type: String::new(),
                                path: "tmp/pkg.tar.gz".to_owned(),
                                size: tar_gz.len() as u64,
                                mode: "-rw-r--r--".to_owned(),
                                uid: 0,
                                gid: 0,
                            }));
                            s_tar.preview_content.set(Some(classify_file_preview(
                                "tmp/pkg.tar.gz",
                                &tar_gz,
                                tar_gz.len() as u64,
                            )));
                        },
                    )),
            )
            .child(file_preview_sheet(state))
    }

    fn screen_contains(screen: &Screen, needle: &str) -> bool {
        screen.texts().iter().any(|t| t.contains(needle))
    }

    #[test]
    fn sheet_rich_previews_support_image_svg_json_elf_gz_and_archive() {
        let clipboard = Arc::new(MemoryClipboard::new(None));
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            clipboard: Some(clipboard.clone()),
            ..EngineConfig::new(windowed_root::<SheetRichModesHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start sheet rich modes harness");

        app.wait_for_within("PNG", Duration::from_secs(2))
            .expect("PNG format badge shown");
        assert!(app.shows("32 × 32"));
        assert!(
            app.screen()
                .accessibility_state("role:button[Copy]")
                .expect("Copy button exists")
                .disabled
        );
        app.press("Hex").expect("switch PNG preview to Hex");
        app.wait_until("PNG hex dump", Duration::from_secs(2), |s| {
            screen_contains(s, "89 50 4e 47")
        })
        .expect("PNG hex dump visible after toggle");
        assert!(
            !app.screen()
                .accessibility_state("role:button[Copy]")
                .expect("Copy enabled in Hex mode")
                .disabled
        );

        app.press("Load SVG").expect("switch target to SVG");
        app.wait_for_within("SVG", Duration::from_secs(2))
            .expect("SVG badge visible");
        app.press("Source").expect("switch SVG to Source XML");
        app.wait_until("SVG XML source", Duration::from_secs(2), |s| {
            screen_contains(s, "<rect")
        })
        .expect("SVG XML source visible");
        app.press("Copy").expect("copy SVG XML");
        assert!(
            clipboard
                .text()
                .as_deref()
                .unwrap_or_default()
                .contains("<svg")
        );

        app.press("Load JSON")
            .expect("switch target to minified JSON");
        app.wait_until("formatted JSON", Duration::from_secs(2), |s| {
            screen_contains(s, "\"version\": \"1.2.0\"")
        })
        .expect("JSON auto-formatted in Formatted mode");
        app.press("Raw").expect("switch JSON to Raw mode");
        app.wait_for_within(
            "{\"name\":\"app\",\"version\":\"1.2.0\"}",
            Duration::from_secs(2),
        )
        .expect("raw JSON visible");

        app.press("Load ELF").expect("switch target to ELF binary");
        app.wait_for_within("x86_64 (AMD64)", Duration::from_secs(2))
            .expect("ELF inspector details visible");
        app.press("Hex").expect("switch ELF inspector to Hex");
        app.wait_until("ELF hex dump", Duration::from_secs(2), |s| {
            screen_contains(s, "7f 45 4c 46")
        })
        .expect("ELF hex dump visible after toggle");

        app.press("Load GZ").expect("switch target to gzipped JS");
        app.wait_until("gzip banner", Duration::from_secs(2), |s| {
            screen_contains(s, "Decompressed from gzip stream")
        })
        .expect("gzip decompression banner shown");
        assert!(screen_contains(app.screen(), "export const ready = true;"));

        app.press("Load TAR")
            .expect("switch target to tar.gz archive");
        app.wait_until("archive banner", Duration::from_secs(2), |s| {
            screen_contains(s, "TAR.GZ · 1 entries")
        })
        .expect("archive summary banner shown");
        assert!(screen_contains(app.screen(), "pkg/readme.txt"));
        app.press("Hex").expect("switch TAR.GZ archive to Hex");
        app.wait_until("archive hex dump", Duration::from_secs(2), |s| {
            screen_contains(s, "1f 8b 08")
        })
        .expect("archive hex dump visible");
    }
}
