//! Tab 4 (`Dockerfile`): reconstructed Dockerfile viewer with Copy, Wrap, Export, and image metadata.

use std::sync::Arc;
use std::time::Duration;

use diving_core::DockerAnalyzeResult;
use nsh_core::prelude::*;

use crate::domain::AppState;
use crate::i18n::{UiLang, t};
use crate::ui::{
    BadgeTone, ButtonVariant, IconName, UiSize, font_mono_family, radius_inset, theme, ui_badge,
    ui_button, ui_code_viewer, ui_empty_state, ui_switch,
};

/// Renders the `Dockerfile` toolbar, image metadata (`ENV` / `LABEL`), and read-only viewer.
pub fn dockerfile_tab(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let lang = state.lang;
    let wrap = state.dockerfile_wrap;
    let copied = signal(false);
    let copy_seq = signal(0_u64);
    let show_meta = signal(false);

    let empty_dockerfile = res.dockerfile.trim().is_empty();
    let dockerfile_for_copy = res.dockerfile.clone();
    let dockerfile_for_export = res.dockerfile.clone();
    let dockerfile_for_view = res.dockerfile.clone();
    let user = res.user.clone();
    let envs = res.envs.clone();
    let labels = res.labels.clone();
    let meta_count = envs.len() + labels.len();
    let export_state = state;

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_col()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .px_4()
                .py_2()
                .bg(move || theme().background)
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .when(!user.is_empty(), |r| {
                            r.child(ui_badge(
                                move || {
                                    SharedString::from(format!(
                                        "{}: {user}",
                                        t(lang.get(), "kpi.user")
                                    ))
                                },
                                BadgeTone::Neutral,
                            ))
                        })
                        .when(meta_count > 0, |r| {
                            r.child(dynamic(move || {
                                let expanded = show_meta.get();
                                ui_button(
                                    move || {
                                        SharedString::from(format!(
                                            "{} ({meta_count})",
                                            t(lang.get(), "dockerfile.metadata")
                                        ))
                                    },
                                    ButtonVariant::Ghost,
                                    UiSize::Sm,
                                    Some(if expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    }),
                                    false,
                                    false,
                                    move || show_meta.set(!show_meta.get_untracked()),
                                )
                                .into_element()
                            }))
                        })
                        .child(ui_switch(
                            move || SharedString::from(t(lang.get(), "dockerfile.wrap")),
                            wrap,
                            empty_dockerfile,
                            move |next| wrap.set(next),
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(dynamic(move || {
                            let is_copied = copied.get();
                            let payload = dockerfile_for_copy.clone();
                            ui_button(
                                move || {
                                    SharedString::from(if copied.get() {
                                        t(lang.get(), "common.copied")
                                    } else {
                                        t(lang.get(), "common.copy")
                                    })
                                },
                                ButtonVariant::Outline,
                                UiSize::Sm,
                                Some(if is_copied {
                                    IconName::Check
                                } else {
                                    IconName::Copy
                                }),
                                empty_dockerfile,
                                false,
                                move || {
                                    if payload.trim().is_empty() {
                                        return;
                                    }
                                    drop(set_clipboard(payload.clone()));
                                    let next_seq = copy_seq.get_untracked().wrapping_add(1);
                                    copy_seq.set(next_seq);
                                    copied.set(true);
                                    set_timeout(Duration::from_millis(1500), move || {
                                        if copy_seq.try_get() == Some(next_seq) {
                                            copied.try_set(false);
                                        }
                                    });
                                },
                            )
                            .into_element()
                        }))
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "dockerfile.export")),
                            ButtonVariant::Outline,
                            UiSize::Sm,
                            Some(IconName::Download),
                            empty_dockerfile,
                            false,
                            move || export_state.export_dockerfile(dockerfile_for_export.clone()),
                        )),
                ),
        )
        .child(when(
            move || show_meta.get() && meta_count > 0,
            move || metadata_panel(envs.clone(), labels.clone(), lang),
        ))
        .child(if empty_dockerfile {
            view()
                .flex_1()
                .min_h(px(0.0))
                .items_center()
                .justify_center()
                .p_4()
                .child(ui_empty_state(
                    IconName::Dockerfile,
                    move || {
                        SharedString::from(match lang.get() {
                            UiLang::En => "No Dockerfile instructions available",
                            UiLang::Zh => "暂无可重建的 Dockerfile 指令",
                        })
                    },
                    move || {
                        SharedString::from(match lang.get() {
                            UiLang::En => "This image contains no recorded layer build commands.",
                            UiLang::Zh => "该镜像未记录可用于重建的镜像层构建指令。",
                        })
                    },
                    None,
                ))
                .into_element()
        } else {
            view()
                .flex_1()
                .min_h(px(0.0))
                .p_4()
                .child(ui_code_viewer(
                    Some(SharedString::from(dockerfile_for_view)),
                    Some(SharedString::from("Dockerfile")),
                    wrap,
                    "Reconstructed Dockerfile viewer",
                ))
                .into_element()
        })
}

fn metadata_panel(
    envs: Vec<String>,
    labels: Vec<String>,
    lang: Signal<UiLang>,
) -> impl IntoElement {
    scroll()
        .max_h(px(220.0))
        .w_full()
        .bg(move || theme().background)
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .w_full()
                .px_4()
                .py_3()
                .flex_col()
                .gap_3()
                .when(!envs.is_empty(), |col| {
                    let count = envs.len();
                    col.child(
                        view()
                            .flex_col()
                            .gap_1p5()
                            .child(
                                text(move || {
                                    format!("{} ({count})", t(lang.get(), "dockerfile.envs"))
                                })
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                            )
                            .children(envs.into_iter().map(|entry| {
                                view()
                                    .px_2()
                                    .py_1()
                                    .rounded(radius_inset)
                                    .bg(move || theme().muted)
                                    .child(
                                        text(entry)
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .text_color(move || theme().foreground),
                                    )
                            })),
                    )
                })
                .when(!labels.is_empty(), |col| {
                    let count = labels.len();
                    col.child(
                        view()
                            .flex_col()
                            .gap_1p5()
                            .child(
                                text(move || {
                                    format!("{} ({count})", t(lang.get(), "dockerfile.labels"))
                                })
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                            )
                            .children(labels.into_iter().map(|entry| {
                                view()
                                    .px_2()
                                    .py_1()
                                    .rounded(radius_inset)
                                    .bg(move || theme().muted)
                                    .child(
                                        text(entry)
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .text_color(move || theme().foreground),
                                    )
                            })),
                    )
                }),
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
    use crate::domain::DivingService;
    use crate::ui::app_tokens;

    fn sample_res(dockerfile: &str) -> Arc<DockerAnalyzeResult> {
        Arc::new(DockerAnalyzeResult {
            dockerfile: dockerfile.to_owned(),
            envs: vec!["PATH=/usr/bin".to_owned()],
            labels: vec!["org.demo=1".to_owned()],
            ..Default::default()
        })
    }

    #[component]
    fn DockerfileHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let mounted = signal(true);
        let empty_mode = signal(false);
        let non_empty = sample_res("FROM alpine:3.20\nCMD [\"sh\"]\n");
        let empty = sample_res("");

        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(ui_button(
                        "Unmount tab",
                        ButtonVariant::Ghost,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        move || mounted.set(false),
                    ))
                    .child(ui_button(
                        "Switch empty",
                        ButtonVariant::Ghost,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        move || empty_mode.set(true),
                    )),
            )
            .child(when(mounted, move || {
                let s = state.clone();
                let ne = non_empty.clone();
                let em = empty.clone();
                dynamic(move || {
                    let res = if empty_mode.get() {
                        em.clone()
                    } else {
                        ne.clone()
                    };
                    dockerfile_tab(s.clone(), res).into_element()
                })
            }))
    }

    #[test]
    fn copy_timer_safe_after_tab_unmount_and_empty_dockerfile_shows_empty_state() {
        let clipboard = Arc::new(MemoryClipboard::new(None));
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            clock: Clock::manual(SystemTime::UNIX_EPOCH),
            clipboard: Some(clipboard.clone()),
            ..EngineConfig::new(windowed_root::<DockerfileHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start dockerfile harness");
        app.wait_for_within("Image Metadata (2)", Duration::from_secs(2))
            .expect("metadata toggle rendered");
        app.press("Image Metadata (2)").expect("expand metadata");
        app.wait_for_within("PATH=/usr/bin", Duration::from_secs(2))
            .expect("env metadata revealed");

        app.press("Copy").expect("copy dockerfile");
        assert!(
            clipboard
                .text()
                .as_deref()
                .is_some_and(|s| s.contains("FROM alpine:3.20"))
        );
        app.wait_for_within("Copied", Duration::from_secs(1))
            .expect("Copied feedback shown");

        app.press("Unmount tab").expect("unmount dockerfile tab");
        app.wait_for_gone("Copied", Duration::from_secs(2))
            .expect("dockerfile tab unmounted");
        app.fast_forward(Duration::from_millis(1600))
            .expect("1.5s copy timer after tab unmount must not panic");

        // Verify empty Dockerfile renders empty state and disables Copy/Export.
        let config_empty = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<DockerfileEmptyHarness>().tokens(app_tokens()))
        };
        let mut app_empty = Headless::start(config_empty).expect("start empty dockerfile harness");
        app_empty
            .wait_for_within(
                "No Dockerfile instructions available",
                Duration::from_secs(2),
            )
            .expect("empty dockerfile state shown");
        assert_eq!(app_empty.screen().enabled("Copy"), Ok(false));
        assert_eq!(app_empty.screen().enabled("Export…"), Ok(false));
    }

    #[component]
    fn DockerfileEmptyHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        assert!(
            state.dockerfile_wrap.get_untracked(),
            "dockerfile_wrap must default to true"
        );
        provide_context(state.clone());
        dockerfile_tab(state, sample_res(""))
    }
}
