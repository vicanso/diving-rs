//! Landing view with composite image search, local source actions, progressive preview, and history.

use nsh_core::prelude::*;

use crate::domain::{AppState, RecentImageEntry, format_bytes};
use crate::features::arch_select_items;
use crate::i18n::{t, tf};
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, UiSize, radius_control, radius_lg, theme,
    ui_badge, ui_banner, ui_button, ui_icon, ui_input_with_ref, ui_panel_titled, ui_progress_bar,
    ui_select, ui_separator, ui_skeleton,
};

const SAMPLE_IMAGES: &[&str] = &[
    "redis:alpine",
    "nginx:alpine",
    "postgres:16-alpine",
    "node:22-alpine",
];

/// Renders the centered Landing workspace.
pub fn landing_view(state: AppState) -> impl IntoElement {
    let err_cond = state.clone();
    let err_view = state.clone();
    let notice_cond = state.clone();
    let notice_view = state.clone();
    let preview_cond = state.clone();
    let preview_view = state.clone();
    let recent_cond = state.clone();
    let recent_view = state.clone();

    scroll().flex_1().child(
        view().w_full().items_center().py_8().px_6().child(
            view()
                .w_full()
                .max_w(px(700.0))
                .flex_col()
                .gap_6()
                .child(hero_header(state.clone()))
                .child(search_card(state.clone()))
                .child(local_source_actions(state.clone()))
                .child(sample_chips(state))
                .child(when(
                    move || err_cond.analyze_error.get().is_some(),
                    move || error_banner(err_view.clone()),
                ))
                .child(when(
                    move || notice_cond.status_notice.get().is_some(),
                    move || {
                        let s = notice_view.clone();
                        ui_banner(
                            BannerTone::Info,
                            move || SharedString::from(s.status_notice.get().unwrap_or_default()),
                            None,
                        )
                    },
                ))
                .child(when(
                    move || {
                        preview_cond.is_loading.get()
                            && (preview_cond.show_preview_card.get()
                                || preview_cond.preview.get().is_some())
                    },
                    move || progressive_preview_card(preview_view.clone()),
                ))
                .child(when(
                    move || !recent_cond.recent_images.get().is_empty(),
                    move || recent_analyses_panel(recent_view.clone()),
                )),
        ),
    )
}

fn hero_header(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    view()
        .flex_col()
        .gap_2()
        .pt_6()
        .child(view().flex_row().child(ui_badge(
            move || SharedString::from(t(lang.get(), "landing.eyebrow")),
            BadgeTone::Neutral,
        )))
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

fn search_card(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let submit_input = state.clone();
    let submit_btn = state.clone();

    view()
        .rounded(radius_lg)
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .p_2()
        .flex_row()
        .items_center()
        .gap_2()
        .child(view().flex_1().min_w(px(0.0)).child(ui_input_with_ref(
            state.image_input,
            move || t(lang.get(), "landing.placeholder").to_owned(),
            "Image reference input",
            UiSize::Lg,
            Some(&state.search_input_ref),
            true,
            Some(Box::new(move |_| submit_input.submit_current_input())),
        )))
        .child(view().w(px(142.0)).flex_shrink_0().child(ui_select(
            move || arch_select_items(lang.get()),
            state.arch_input,
            move || SharedString::from(t(lang.get(), "landing.arch_auto")),
            "Target architecture",
            UiSize::Lg,
            state.is_loading,
            move |next| state.arch_input.set(next.filter(|s| !s.is_empty())),
        )))
        .child(ui_button(
            move || {
                SharedString::from(if submit_btn.is_loading.get() {
                    t(lang.get(), "landing.analyzing")
                } else {
                    t(lang.get(), "landing.analyze")
                })
            },
            ButtonVariant::Primary,
            UiSize::Lg,
            Some(IconName::Search),
            false,
            state.is_loading,
            move || submit_btn.submit_current_input(),
        ))
}

fn local_source_actions(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let file_state = state.clone();
    let local_state = state.clone();

    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(ui_button(
            move || SharedString::from(t(lang.get(), "menu.open_local_file")),
            ButtonVariant::Outline,
            UiSize::Md,
            Some(IconName::FolderInput),
            false,
            false,
            move || file_state.open_local_file_dialog(),
        ))
        .child(ui_button(
            move || SharedString::from(t(lang.get(), "menu.browse_local_images")),
            ButtonVariant::Outline,
            UiSize::Md,
            Some(IconName::Container),
            false,
            false,
            move || local_state.open_local_picker(),
        ))
}

fn sample_chips(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(
            text(move || format!("{}:", t(lang.get(), "landing.samples")))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .children(SAMPLE_IMAGES.iter().map(|sample| {
            let click_state = state.clone();
            let image_name = *sample;
            ui_button(
                image_name,
                ButtonVariant::Outline,
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
        move || SharedString::from(state.analyze_error.get().unwrap_or_default()),
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
                ButtonVariant::Outline,
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
                        .gap_2()
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

fn recent_analyses_panel(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let recent = state.recent_images;

    ui_panel_titled(
        move || SharedString::from(t(lang.get(), "landing.recent")),
        None,
        view().flex_col().gap_1().child(each(
            move || recent.get(),
            |entry: &RecentImageEntry| format!("{}@{}", entry.image, entry.arch),
            move |entry, idx| {
                let row_state = state.clone();
                let snapshot = entry.get_untracked();
                let entry_for_click = snapshot.clone();
                let entry_name = snapshot.image;
                let entry_arch = snapshot.arch;
                let entry_size = format_bytes(snapshot.total_size);
                let entry_score = format!("{}%", snapshot.score);
                let score_tone = if snapshot.score >= 95 {
                    BadgeTone::Success
                } else if snapshot.score >= 85 {
                    BadgeTone::Warning
                } else {
                    BadgeTone::Danger
                };

                view()
                    .flex_col()
                    .child(when(move || idx.get() > 0, ui_separator))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .px_2p5()
                            .py_2()
                            .rounded(radius_control)
                            .hover(|s| s.bg(move || theme().accent))
                            .on_press(move || {
                                let arch = if entry_for_click.arch.is_empty() {
                                    None
                                } else {
                                    Some(entry_for_click.arch.clone())
                                };
                                row_state.analyze(entry_for_click.image.clone(), arch);
                            })
                            .child(
                                view()
                                    .flex_row()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .items_center()
                                    .gap_2()
                                    .child(ui_icon(IconName::Layers, px(14.0), move || {
                                        theme().muted_foreground
                                    }))
                                    .child(
                                        text(entry_name)
                                            .text_sm()
                                            .number_of_lines(1)
                                            .text_color(move || theme().foreground),
                                    )
                                    .when(!entry_arch.is_empty(), |row| {
                                        row.child(ui_badge(entry_arch, BadgeTone::Neutral))
                                    }),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2p5()
                                    .flex_shrink_0()
                                    .child(
                                        text(entry_size)
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                    )
                                    .child(ui_badge(entry_score, score_tone)),
                            ),
                    )
            },
        )),
    )
}
