//! Local Docker / Podman container image picker dialog with daemon failure recovery.

use std::sync::Arc;

use diving_core::{LocalContainerImage, LocalRuntime};
use nsh_core::prelude::*;

use crate::domain::AppState;
use crate::i18n::t;
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, SegmentOption, UiSize, font_mono_family,
    radius_inset, surface_active, theme, ui_badge, ui_banner, ui_button, ui_dialog, ui_empty_state,
    ui_icon_button, ui_search_input, ui_segmented, ui_skeleton,
};

/// Modal centered dialog listing images stored in local Docker or Podman daemons.
pub fn local_picker_dialog(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let body_state = state.clone();
    let footer_state = state.clone();

    ui_dialog(
        state.local_picker_open,
        move || SharedString::from(t(lang.get(), "local.title")),
        px(640.0),
        move || picker_body(body_state.clone()).into_element(),
        move || picker_footer(footer_state.clone()).into_element(),
    )
}

fn picker_body(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let rt_state = state.clone();
    let refresh_state = state.clone();
    let list_state = state.clone();

    view()
        .flex_col()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(ui_segmented(
                    vec![
                        SegmentOption::new(LocalRuntime::Docker, move || {
                            SharedString::from(t(lang.get(), "local.docker"))
                        }),
                        SegmentOption::new(LocalRuntime::Podman, move || {
                            SharedString::from(t(lang.get(), "local.podman"))
                        }),
                    ],
                    state.local_runtime,
                    UiSize::Sm,
                    "Container runtime",
                    move |next| rt_state.set_local_runtime(next),
                ))
                .child(view().flex_1().min_w(px(0.0)).child(ui_search_input(
                    state.local_filter,
                    move || t(lang.get(), "local.filter_placeholder").to_owned(),
                    "Filter local container images",
                    UiSize::Sm,
                    None,
                    None,
                    false,
                    None,
                )))
                .child(ui_icon_button(
                    IconName::Refresh,
                    "Refresh local images",
                    Some("Refresh"),
                    ButtonVariant::Outline,
                    UiSize::Sm,
                    false,
                    state.local_loading,
                    move || refresh_state.refresh_local_images(),
                )),
        )
        .child(
            view()
                .h(px(300.0))
                .flex_col()
                .rounded(radius_inset)
                .border_1()
                .border_color(move || theme().border)
                .overflow_hidden()
                .child(dynamic(move || {
                    if list_state.local_loading.get() {
                        return view()
                            .flex_1()
                            .p_4()
                            .flex_col()
                            .gap_3()
                            .child(
                                text(move || t(lang.get(), "local.loading"))
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(ui_skeleton(px(520.0), px(22.0)))
                            .child(ui_skeleton(px(480.0), px(22.0)))
                            .child(ui_skeleton(px(500.0), px(22.0)))
                            .child(ui_skeleton(px(450.0), px(22.0)))
                            .into_element();
                    }

                    if let Some(err) = list_state.local_error.get() {
                        let cur_rt = list_state.local_runtime.get();
                        let alt_rt = match cur_rt {
                            LocalRuntime::Docker => LocalRuntime::Podman,
                            LocalRuntime::Podman => LocalRuntime::Docker,
                        };
                        let switch_key = match alt_rt {
                            LocalRuntime::Docker => "local.switch_docker",
                            LocalRuntime::Podman => "local.switch_podman",
                        };
                        let switch_state = list_state.clone();
                        let file_state = list_state.clone();

                        return view()
                            .flex_1()
                            .p_4()
                            .flex_col()
                            .gap_3()
                            .child(ui_banner(BannerTone::Danger, SharedString::from(err), None))
                            .child(ui_empty_state(
                                IconName::Container,
                                move || SharedString::from(t(lang.get(), "local.daemon_err_title")),
                                "",
                                Some(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(ui_button(
                                            move || SharedString::from(t(lang.get(), switch_key)),
                                            ButtonVariant::Outline,
                                            UiSize::Sm,
                                            None,
                                            false,
                                            false,
                                            move || switch_state.set_local_runtime(alt_rt),
                                        ))
                                        .child(ui_button(
                                            move || {
                                                SharedString::from(t(
                                                    lang.get(),
                                                    "menu.open_local_file",
                                                ))
                                            },
                                            ButtonVariant::Primary,
                                            UiSize::Sm,
                                            Some(IconName::FolderInput),
                                            false,
                                            false,
                                            move || {
                                                file_state.local_picker_open.set(false);
                                                file_state.open_local_file_dialog();
                                            },
                                        ))
                                        .into_element(),
                                ),
                            ))
                            .into_element();
                    }

                    let keyword = list_state.local_filter.get().trim().to_ascii_lowercase();
                    let filtered: Vec<LocalContainerImage> = list_state
                        .local_images
                        .get()
                        .into_iter()
                        .filter(|img| {
                            keyword.is_empty()
                                || img.repository.to_ascii_lowercase().contains(&keyword)
                                || img.tag.to_ascii_lowercase().contains(&keyword)
                                || img.id.to_ascii_lowercase().contains(&keyword)
                        })
                        .collect();

                    if filtered.is_empty() {
                        return view()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .child(ui_empty_state(
                                IconName::Container,
                                move || SharedString::from(t(lang.get(), "local.empty")),
                                "",
                                None,
                            ))
                            .into_element();
                    }

                    let rows = Arc::new(filtered);
                    let row_state = list_state.clone();
                    let selected_ref = list_state.local_selected_ref;

                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .flex_col()
                        .child(
                            view()
                                .h(px(28.0))
                                .px_3()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .bg(move || theme().muted)
                                .border_b_1()
                                .border_color(move || theme().border)
                                .child(
                                    text(move || t(lang.get(), "local.col_repo"))
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(move || t(lang.get(), "local.col_id"))
                                        .w(px(108.0))
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(move || t(lang.get(), "local.col_created"))
                                        .w(px(110.0))
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(move || t(lang.get(), "local.col_size"))
                                        .w(px(76.0))
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_right()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(
                            virtual_list(
                                {
                                    let rows = Arc::clone(&rows);
                                    move || rows.len()
                                },
                                {
                                    let rows = Arc::clone(&rows);
                                    move |at| {
                                        let item = &rows[at.index()];
                                        let reference = item.reference();
                                        let repo_label =
                                            if item.tag.is_empty() || item.tag == "<none>" {
                                                item.repository.clone()
                                            } else {
                                                format!("{}:{}", item.repository, item.tag)
                                            };
                                        let id_short = item.id.clone();
                                        let created = item.created.clone();
                                        let size = item.size.clone();
                                        let click_ref = reference.clone();
                                        let click_state = row_state.clone();
                                        let row_ref = reference.clone();

                                        view()
                                            .w_full()
                                            .min_h(px(32.0))
                                            .px_3()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .border_b_1()
                                            .border_color(move || theme().border.opacity(0.45))
                                            .bg(move || {
                                                if selected_ref.get().as_deref() == Some(&row_ref) {
                                                    surface_active()
                                                } else {
                                                    theme().card
                                                }
                                            })
                                            .hover(|s| s.bg(move || theme().accent))
                                            .on_press(move || {
                                                click_state
                                                    .local_selected_ref
                                                    .set(Some(click_ref.clone()));
                                            })
                                            .child(
                                                text(repo_label)
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .number_of_lines(1)
                                                    .text_color(move || theme().foreground),
                                            )
                                            .child(
                                                view()
                                                    .w(px(108.0))
                                                    .child(ui_badge(id_short, BadgeTone::Neutral)),
                                            )
                                            .child(
                                                text(created)
                                                    .w(px(110.0))
                                                    .text_xs()
                                                    .number_of_lines(1)
                                                    .text_color(move || theme().muted_foreground),
                                            )
                                            .child(
                                                text(size)
                                                    .w(px(76.0))
                                                    .text_xs()
                                                    .font_family(font_mono_family())
                                                    .tabular_nums()
                                                    .text_right()
                                                    .text_color(move || theme().foreground),
                                            )
                                    }
                                },
                            )
                            .flex_1()
                            .min_h(px(0.0))
                            .estimated_item_size(px(32.0))
                            .keys(move || {
                                rows.iter()
                                    .map(|item| item.reference())
                                    .collect::<Vec<String>>()
                            }),
                        )
                        .into_element()
                })),
        )
}

fn picker_footer(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let cancel_state = state.clone();
    let analyze_state = state.clone();
    let no_selection = move || state.local_selected_ref.get().is_none();

    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text(move || t(lang.get(), "local.cli_hint"))
                .flex_1()
                .min_w(px(0.0))
                .text_xs()
                .number_of_lines(1)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(ui_button(
                    move || SharedString::from(t(lang.get(), "landing.cancel")),
                    ButtonVariant::Ghost,
                    UiSize::Md,
                    None,
                    false,
                    false,
                    move || cancel_state.local_picker_open.set(false),
                ))
                .child(ui_button(
                    move || SharedString::from(t(lang.get(), "local.analyze_selected")),
                    ButtonVariant::Primary,
                    UiSize::Md,
                    Some(IconName::Search),
                    no_selection,
                    false,
                    move || analyze_state.analyze_selected_local_image(),
                )),
        )
}
