//! Right-edge File Preview Sheet (`CodeViewer` for UTF-8 text, hex dump for binary files).

use std::time::Duration;

use nsh_core::prelude::*;

use crate::domain::{AppState, FilePreviewContent, format_bytes};
use crate::i18n::{format_user_error, t, tf};
use crate::status_notice_banner;
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, UiSize, theme, ui_badge, ui_banner, ui_button,
    ui_code_viewer, ui_empty_state, ui_hex_viewer, ui_separator, ui_sheet, ui_skeleton,
};

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
    let header_state = state.clone();
    let notice_state = state.clone();
    let content_state = state.clone();
    let target_reset_state = state.clone();

    effect(move || {
        let _ = target_reset_state.preview_target.get();
        copy_seq.set(copy_seq.get_untracked().wrapping_add(1));
        copied.set(false);
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
                    let is_symlink = content_state
                        .preview_target
                        .get()
                        .is_some_and(|target| target.mode.starts_with('l'));
                    match content_state.preview_content.get() {
                        Some(FilePreviewContent::Empty) => ui_empty_state(
                            IconName::File,
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
                                let shown = format_bytes(shown_bytes);
                                let total = format_bytes(total_size);
                                col.child(ui_banner(
                                    BannerTone::Info,
                                    move || {
                                        SharedString::from(tf(
                                            lang.get(),
                                            "sheet.truncated",
                                            &[&shown, &total],
                                        ))
                                    },
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
                        }) => {
                            let shown = format_bytes(shown_bytes);
                            let total = format_bytes(total_size);
                            view()
                                .flex_1()
                                .min_h(px(0.0))
                                .flex_col()
                                .gap_2()
                                .child(ui_banner(
                                    BannerTone::Info,
                                    move || {
                                        SharedString::from(tf(
                                            lang.get(),
                                            "sheet.binary",
                                            &[&shown, &total],
                                        ))
                                    },
                                    None,
                                ))
                                .child(ui_hex_viewer(
                                    Some(SharedString::from(hex_dump)),
                                    "Binary file hex dump viewer",
                                ))
                                .into_element()
                        }
                        None => view().into_element(),
                    }
                })),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use diving_core::Analyzer;
    use nsh_core::clipboard::{ClipboardBackend, MemoryClipboard};
    use nsh_engine::{Clock, EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::domain::{DivingService, PreviewTarget};
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
            language: Some("ini"),
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
                            language: None,
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

        app.press("Close sheet").expect("close sheet before switch");
        app.wait_for_gone("00000000  7f 45 4c 46  |.ELF|", Duration::from_secs(2))
            .expect("sheet closed");
        app.press("Switch to symlink").expect("switch to symlink");
        app.wait_for_within("Symbolic link → busybox", Duration::from_secs(2))
            .expect("symlink target displayed cleanly");
        assert!(app.shows("Symbolic link"));
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
}
