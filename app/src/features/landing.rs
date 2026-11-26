//! Minimal desktop workbench launcher: seamless unified command bar, tonal quick actions, and progressive preview.

use nsh_core::prelude::*;

use crate::domain::{AppState, format_bytes};
use crate::features::arch_select_items;
use crate::i18n::{format_user_error, t, tf};
use crate::ui::{
    BannerTone, ButtonVariant, IconName, UiSize, border_strong, font_mono_family, radius_control,
    radius_lg, text_tertiary, theme, ui_action_card, ui_banner, ui_bare_input_with_ref,
    ui_borderless_select, ui_button, ui_icon, ui_panel_titled, ui_progress_bar, ui_skeleton,
};

const SAMPLE_IMAGES: &[&str] = &[
    "redis:alpine",
    "nginx:alpine",
    "postgres:16-alpine",
    "node:22-alpine",
];

/// Renders the calm centered Desktop Launcher in the main stage.
pub fn landing_view(state: AppState) -> impl IntoElement {
    let err_cond = state.clone();
    let err_view = state.clone();
    let preview_cond = state.clone();
    let preview_view = state.clone();

    scroll().flex_1().child(
        view()
            .w_full()
            .items_center()
            .pt(move || {
                if window().height < px(700.0) {
                    px(36.0)
                } else {
                    px(88.0)
                }
            })
            .pb_8()
            .px_6()
            .child(
                view()
                    .w_full()
                    .max_w(px(620.0))
                    .flex_col()
                    .gap_5()
                    .child(launcher_heading(state.clone()))
                    .child(unified_command_bar(state.clone()))
                    .child(quick_action_grid(state.clone()))
                    .child(sample_strip(state))
                    .child(when(
                        move || err_cond.analyze_error.get().is_some(),
                        move || error_banner(err_view.clone()),
                    ))
                    .child(when(
                        move || {
                            preview_cond.is_loading.get()
                                && (preview_cond.show_preview_card.get()
                                    || preview_cond.preview.get().is_some())
                        },
                        move || progressive_preview_card(preview_view.clone()),
                    )),
            ),
    )
}

fn launcher_heading(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    view()
        .flex_col()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2p5()
                .child(
                    view()
                        .size(px(28.0))
                        .items_center()
                        .justify_center()
                        .rounded(radius_control)
                        .bg(move || theme().primary.opacity(0.12))
                        .child(ui_icon(IconName::Logo, px(16.0), move || theme().primary)),
                )
                .child(
                    text(move || t(lang.get(), "landing.eyebrow"))
                        .text_xs()
                        .font_family(font_mono_family())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            text(move || t(lang.get(), "landing.title"))
                .text_2xl()
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(move || t(lang.get(), "landing.lead"))
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
}

fn unified_command_bar(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let submit_input = state.clone();
    let submit_btn = state.clone();

    view()
        .w_full()
        .pl_3()
        .pr_1p5()
        .py_1p5()
        .rounded(radius_lg)
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().input)
        .shadow_sm()
        .flex_row()
        .items_center()
        .gap_2()
        .transition_colors()
        .hover(|s| s.border_color(border_strong))
        .focus_within(|s| s.border_color(move || theme().ring))
        .child(ui_icon(IconName::Search, px(16.0), move || {
            theme().muted_foreground
        }))
        .child(view().flex_1().min_w(px(0.0)).child(ui_bare_input_with_ref(
            state.image_input,
            move || t(lang.get(), "landing.placeholder").to_owned(),
            "Image reference input",
            UiSize::Lg,
            Some(&state.search_input_ref),
            true,
            Some(Box::new(move |_| submit_input.submit_current_input())),
        )))
        .child(
            view()
                .w(px(1.0))
                .h(px(20.0))
                .bg(move || theme().border)
                .accessibility_elements_hidden(true),
        )
        .child(
            view()
                .w(px(122.0))
                .flex_shrink_0()
                .child(ui_borderless_select(
                    move || arch_select_items(lang.get()),
                    state.arch_input,
                    move || SharedString::from(t(lang.get(), "landing.arch_auto")),
                    "Target architecture",
                    UiSize::Md,
                    state.is_loading,
                    move |next| state.arch_input.set(next.filter(|s| !s.is_empty())),
                )),
        )
        .child(ui_button(
            move || {
                SharedString::from(if submit_btn.is_loading.get() {
                    t(lang.get(), "landing.analyzing")
                } else {
                    t(lang.get(), "landing.analyze")
                })
            },
            ButtonVariant::Primary,
            UiSize::Md,
            None,
            false,
            state.is_loading,
            move || submit_btn.submit_current_input(),
        ))
}

fn quick_action_grid(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let file_state = state.clone();
    let local_state = state.clone();
    let compare_state = state;

    view()
        .w_full()
        .flex_row()
        .items_stretch()
        .gap_2p5()
        .child(ui_action_card(
            move || SharedString::from(t(lang.get(), "menu.open_local_file")),
            move || SharedString::from(t(lang.get(), "landing.quick_local_file_desc")),
            IconName::FolderInput,
            Some("mod+o"),
            move || file_state.open_local_file_dialog(),
        ))
        .child(ui_action_card(
            move || SharedString::from(t(lang.get(), "menu.browse_local_images")),
            move || SharedString::from(t(lang.get(), "landing.quick_docker_desc")),
            IconName::Container,
            Some("mod+shift+o"),
            move || local_state.open_local_picker(),
        ))
        .child(ui_action_card(
            move || SharedString::from(t(lang.get(), "menu.compare_with")),
            move || SharedString::from(t(lang.get(), "landing.quick_compare_desc")),
            IconName::Compare,
            Some("mod+d"),
            move || compare_state.open_compare_dialog(),
        ))
}

fn sample_strip(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .child(
            text(move || format!("{}:", t(lang.get(), "landing.samples")))
                .text_xs()
                .font_family(font_mono_family())
                .text_color(text_tertiary),
        )
        .children(SAMPLE_IMAGES.iter().map(|sample| {
            let click_state = state.clone();
            let image_name = *sample;
            ui_button(
                image_name,
                ButtonVariant::Ghost,
                UiSize::Xs,
                None,
                state.is_loading,
                false,
                move || click_state.analyze(image_name.to_owned(), None),
            )
        }))
}

fn error_banner(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let retry_state = state.clone();
    ui_banner(
        BannerTone::Danger,
        move || {
            let raw = state.analyze_error.get().unwrap_or_default();
            SharedString::from(format_user_error(lang.get(), &raw))
        },
        Some(
            ui_button(
                move || SharedString::from(t(lang.get(), "landing.retry")),
                ButtonVariant::Outline,
                UiSize::Xs,
                Some(IconName::Refresh),
                false,
                false,
                move || retry_state.submit_current_input(),
            )
            .into_element(),
        ),
    )
}

fn progressive_preview_card(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let cancel_state = state.clone();
    let m = motion().entering(MotionPreset::FadeInUp);

    view().bind_motion(&m).child(ui_panel_titled(
        move || SharedString::from(t(lang.get(), "preview.title")),
        Some(
            ui_button(
                move || SharedString::from(t(lang.get(), "landing.cancel")),
                ButtonVariant::Ghost,
                UiSize::Xs,
                Some(IconName::Close),
                false,
                false,
                move || cancel_state.cancel_analysis(),
            )
            .into_element(),
        ),
        view()
            .flex_col()
            .gap_3()
            .child(ui_progress_bar(
                state.progress_ratio,
                state.progress_indeterminate,
                "Image analysis progress",
            ))
            .child(
                text(move || {
                    state
                        .progress_text
                        .get()
                        .unwrap_or_else(|| t(lang.get(), "landing.analyzing").to_owned())
                })
                .text_xs()
                .text_color(move || theme().muted_foreground),
            )
            .child(dynamic(move || {
                if let Some(prev) = state.preview.get() {
                    let meta = tf(
                        lang.get(),
                        "preview.meta",
                        &[
                            &format!("{}/{}", prev.os, prev.arch),
                            &prev.layers.len().to_string(),
                            &format_bytes(prev.size),
                        ],
                    );
                    view()
                        .flex_col()
                        .gap_1p5()
                        .child(
                            text(meta)
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(move || t(lang.get(), "preview.hint"))
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .into_element()
                } else {
                    view()
                        .flex_col()
                        .gap_2()
                        .child(ui_skeleton(px(260.0), px(12.0)))
                        .child(ui_skeleton(px(190.0), px(12.0)))
                        .into_element()
                }
            })),
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::domain::DivingService;
    use crate::i18n::UiLang;
    use crate::ui::app_tokens;

    #[component]
    fn LandingHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state
            .analyze_error
            .set(Some("Tar fail: unexpected EOF".to_owned()));
        let zh_state = state.clone();
        view()
            .flex_1()
            .flex_col()
            .child(ui_button(
                "Switch ZH",
                ButtonVariant::Outline,
                UiSize::Xs,
                None,
                false,
                false,
                move || zh_state.set_lang(UiLang::Zh),
            ))
            .child(landing_view(state))
    }

    #[test]
    fn landing_renders_eyebrow_lead_and_friendly_tar_error() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<LandingHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start landing harness");
        app.wait_for_within("Docker · OCI · layers", Duration::from_secs(3))
            .expect("landing eyebrow rendered");
        assert!(app.shows("Inspect container image"));
        assert!(app.shows("Inspect layer waste, file trees, secrets, and runtime compatibility across Docker/OCI images."));
        assert!(app.shows("Unable to read local archive: unexpected EOF"));
        assert!(!app.shows("Tar fail:"));

        app.press("Switch ZH").expect("toggle ZH");
        app.wait_for_within(
            "无法读取本地镜像归档：unexpected EOF",
            Duration::from_secs(2),
        )
        .expect("landing error banner re-localizes live on EN/ZH toggle");
    }
}
