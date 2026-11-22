//! Local Docker / Podman container image picker dialog with daemon failure recovery.

use std::time::Duration;

use diving_core::{LocalContainerImage, LocalRuntime};
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::domain::{AppState, SharedRef};
use crate::i18n::{format_user_error, t};
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, SegmentOption, UiSize, font_mono_family,
    radius_inset, surface_active, theme, ui_badge, ui_banner, ui_button, ui_dialog, ui_empty_state,
    ui_icon_button, ui_search_input_with_escape, ui_segmented, ui_skeleton,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerViewState {
    Loading,
    Error,
    Empty,
    List,
}

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

fn step_local_selection(
    rows: &Memo<SharedRef<Vec<LocalContainerImage>>>,
    state: &AppState,
    list_ref: &ElementRef,
    delta: isize,
) {
    if !state.local_picker_open.get_untracked() || state.command_palette_open.get_untracked() {
        return;
    }
    let list = rows.get_untracked();
    if list.is_empty() {
        return;
    }
    let cur_pos = state
        .local_selected_ref
        .get_untracked()
        .and_then(|sel| list.iter().position(|img| img.reference() == sel));
    let next_pos = match cur_pos {
        Some(pos) => (pos as isize + delta).clamp(0, (list.len() - 1) as isize) as usize,
        None if delta >= 0 => 0,
        None => list.len() - 1,
    };
    if let Some(item) = list.get(next_pos) {
        state.local_selected_ref.set(Some(item.reference()));
        list_ref.scroll_to_index(next_pos as u32, Percentage(0.5), px(0.0), false);
    }
}

fn picker_body(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let rt_state = state.clone();
    let refresh_state = state.clone();
    let submit_state = state.clone();
    let esc_state = state.clone();

    let filter_ref = element_ref();
    let list_ref = element_ref();
    let focus_target = filter_ref;
    set_timeout(Duration::from_millis(20), move || {
        focus_target.focus();
    });

    let filter_sig = state.local_filter;
    let images_sig = state.local_images;
    let loading_sig = state.local_loading;
    let error_sig = state.local_error;
    let selected_ref = state.local_selected_ref;

    let filtered_rows: Memo<SharedRef<Vec<LocalContainerImage>>> = computed(move || {
        let keyword = filter_sig.get().trim().to_ascii_lowercase();
        let filtered: Vec<LocalContainerImage> = images_sig
            .get()
            .into_iter()
            .filter(|img| {
                keyword.is_empty()
                    || img.repository.to_ascii_lowercase().contains(&keyword)
                    || img.tag.to_ascii_lowercase().contains(&keyword)
                    || img.id.to_ascii_lowercase().contains(&keyword)
            })
            .collect();
        SharedRef::new(filtered)
    });

    effect(move || {
        let rows = filtered_rows.get();
        if let Some(cur) = selected_ref.get_untracked()
            && !rows.iter().any(|img| img.reference() == cur)
        {
            selected_ref.set(None);
        }
    });

    let view_state = computed(move || {
        if loading_sig.get() {
            PickerViewState::Loading
        } else if error_sig.get().is_some() {
            PickerViewState::Error
        } else if filtered_rows.with(|r| r.is_empty()) {
            PickerViewState::Empty
        } else {
            PickerViewState::List
        }
    });

    let nav_options = KeyOptions {
        when: When::Always,
        repeat: true,
        ..Default::default()
    };
    let up_state = state.clone();
    let up_rows = filtered_rows;
    let up_list = list_ref;
    on_key_with("up", nav_options, move |_| {
        step_local_selection(&up_rows, &up_state, &up_list, -1);
    });
    let down_state = state.clone();
    let down_rows = filtered_rows;
    let down_list = list_ref;
    on_key_with("down", nav_options, move |_| {
        step_local_selection(&down_rows, &down_state, &down_list, 1);
    });

    let list_state = state;

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
                    list_state.local_runtime,
                    UiSize::Sm,
                    "Container runtime",
                    move |next| rt_state.set_local_runtime(next),
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(ui_search_input_with_escape(
                            filter_sig,
                            move || t(lang.get(), "local.filter_placeholder").to_owned(),
                            "Filter local container images",
                            UiSize::Sm,
                            None,
                            Some(&filter_ref),
                            false,
                            Some(Box::new(move |_| {
                                submit_state.analyze_selected_local_image()
                            })),
                            Some(Box::new(move || esc_state.local_picker_open.set(false))),
                        )),
                )
                .child(ui_icon_button(
                    IconName::Refresh,
                    "Refresh local images",
                    Some(move || SharedString::from(t(lang.get(), "local.refresh"))),
                    ButtonVariant::Outline,
                    UiSize::Sm,
                    false,
                    loading_sig,
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
                .child(dynamic(move || match view_state.get() {
                    PickerViewState::Loading => view()
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
                        .into_element(),
                    PickerViewState::Error => {
                        let switch_state = list_state.clone();
                        let file_state = list_state.clone();
                        let folder_state = list_state.clone();

                        view()
                            .flex_1()
                            .flex_col()
                            .child(view().px_4().pt_4().child(ui_banner(
                                BannerTone::Danger,
                                move || {
                                    let raw = error_sig.get().unwrap_or_default();
                                    SharedString::from(format_user_error(lang.get(), &raw))
                                },
                                None,
                            )))
                            .child(
                                view().flex_1().items_center().justify_center().child(
                                    ui_empty_state(
                                        IconName::Container,
                                        move || {
                                            SharedString::from(t(
                                                lang.get(),
                                                "local.daemon_err_title",
                                            ))
                                        },
                                        "",
                                        Some(
                                            view()
                                                .flex_row()
                                                .flex_wrap()
                                                .justify_center()
                                                .items_center()
                                                .gap_2()
                                                .child(ui_button(
                                                    move || {
                                                        let alt_key =
                                                            match list_state.local_runtime.get() {
                                                                LocalRuntime::Docker => {
                                                                    "local.switch_podman"
                                                                }
                                                                LocalRuntime::Podman => {
                                                                    "local.switch_docker"
                                                                }
                                                            };
                                                        SharedString::from(t(lang.get(), alt_key))
                                                    },
                                                    ButtonVariant::Outline,
                                                    UiSize::Sm,
                                                    None,
                                                    false,
                                                    false,
                                                    move || {
                                                        let alt = match switch_state
                                                            .local_runtime
                                                            .get_untracked()
                                                        {
                                                            LocalRuntime::Docker => {
                                                                LocalRuntime::Podman
                                                            }
                                                            LocalRuntime::Podman => {
                                                                LocalRuntime::Docker
                                                            }
                                                        };
                                                        switch_state.set_local_runtime(alt);
                                                    },
                                                ))
                                                .child(ui_button(
                                                    move || {
                                                        SharedString::from(t(
                                                            lang.get(),
                                                            "menu.open_local_file",
                                                        ))
                                                    },
                                                    ButtonVariant::Outline,
                                                    UiSize::Sm,
                                                    Some(IconName::FolderInput),
                                                    false,
                                                    false,
                                                    move || {
                                                        file_state.local_picker_open.set(false);
                                                        file_state.open_local_file_dialog();
                                                    },
                                                ))
                                                .child(ui_button(
                                                    move || {
                                                        SharedString::from(t(
                                                            lang.get(),
                                                            "action.open_folder",
                                                        ))
                                                    },
                                                    ButtonVariant::Outline,
                                                    UiSize::Sm,
                                                    Some(IconName::FolderOpen),
                                                    false,
                                                    false,
                                                    move || {
                                                        folder_state.local_picker_open.set(false);
                                                        folder_state.open_local_folder();
                                                    },
                                                ))
                                                .into_element(),
                                        ),
                                    ),
                                ),
                            )
                            .into_element()
                    }
                    PickerViewState::Empty => {
                        let file_state = list_state.clone();
                        let folder_state = list_state.clone();
                        view()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .child(ui_empty_state(
                                IconName::Container,
                                move || SharedString::from(t(lang.get(), "local.empty")),
                                "",
                                Some(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(ui_button(
                                            move || {
                                                SharedString::from(t(
                                                    lang.get(),
                                                    "menu.open_local_file",
                                                ))
                                            },
                                            ButtonVariant::Outline,
                                            UiSize::Sm,
                                            Some(IconName::FolderInput),
                                            false,
                                            false,
                                            move || {
                                                file_state.local_picker_open.set(false);
                                                file_state.open_local_file_dialog();
                                            },
                                        ))
                                        .child(ui_button(
                                            move || {
                                                SharedString::from(t(
                                                    lang.get(),
                                                    "action.open_folder",
                                                ))
                                            },
                                            ButtonVariant::Outline,
                                            UiSize::Sm,
                                            Some(IconName::FolderOpen),
                                            false,
                                            false,
                                            move || {
                                                folder_state.local_picker_open.set(false);
                                                folder_state.open_local_folder();
                                            },
                                        ))
                                        .into_element(),
                                ),
                            ))
                            .into_element()
                    }
                    PickerViewState::List => {
                        let row_state = list_state.clone();

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
                                    move || filtered_rows.with(|r| r.len()),
                                    move |at| {
                                        let click_state = row_state.clone();
                                        let key_state = row_state.clone();

                                        view()
                                            .w_full()
                                            .h(px(32.0))
                                            .px_3()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .role(AccessibilityRole::Button)
                                            .focusable(true)
                                            .focus_visible(|s| {
                                                s.outline(px(2.0))
                                                    .outline_color(move || theme().ring)
                                            })
                                            .border_b_1()
                                            .border_color(move || theme().border.opacity(0.45))
                                            .bg(move || {
                                                let row_ref = filtered_rows.with(|r| {
                                                    r.get(at.index())
                                                        .map(LocalContainerImage::reference)
                                                });
                                                if row_ref.is_some()
                                                    && row_ref == selected_ref.get()
                                                {
                                                    surface_active()
                                                } else {
                                                    theme().card
                                                }
                                            })
                                            .hover(|s| s.bg(move || theme().accent))
                                            .on_key_down(move |ev: &KeyEvent| {
                                                if matches!(ev.key.as_str(), "Enter" | " ")
                                                    && let Some(reference) =
                                                        filtered_rows.with(|r| {
                                                            r.get(at.index())
                                                                .map(LocalContainerImage::reference)
                                                        })
                                                {
                                                    key_state
                                                        .local_selected_ref
                                                        .set(Some(reference));
                                                    if ev.key.as_str() == "Enter" {
                                                        key_state.analyze_selected_local_image();
                                                    }
                                                }
                                            })
                                            .on_press(move || {
                                                if let Some(reference) = filtered_rows.with(|r| {
                                                    r.get(at.index())
                                                        .map(LocalContainerImage::reference)
                                                }) {
                                                    click_state
                                                        .local_selected_ref
                                                        .set(Some(reference));
                                                }
                                            })
                                            .child(
                                                text(move || {
                                                    filtered_rows.with(|r| {
                                                        r.get(at.index()).map_or_else(
                                                            String::new,
                                                            |item| {
                                                                if item.tag.is_empty()
                                                                    || item.tag == "<none>"
                                                                {
                                                                    item.repository.clone()
                                                                } else {
                                                                    format!(
                                                                        "{}:{}",
                                                                        item.repository, item.tag
                                                                    )
                                                                }
                                                            },
                                                        )
                                                    })
                                                })
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .number_of_lines(1)
                                                .text_color(move || theme().foreground),
                                            )
                                            .child(view().w(px(108.0)).items_start().child(
                                                ui_badge(
                                                    move || {
                                                        SharedString::from(filtered_rows.with(
                                                            |r| {
                                                                r.get(at.index()).map_or_else(
                                                                    String::new,
                                                                    |item| item.id.clone(),
                                                                )
                                                            },
                                                        ))
                                                    },
                                                    BadgeTone::Neutral,
                                                ),
                                            ))
                                            .child(
                                                text(move || {
                                                    filtered_rows.with(|r| {
                                                        r.get(at.index())
                                                            .map_or_else(String::new, |item| {
                                                                item.created.clone()
                                                            })
                                                    })
                                                })
                                                .w(px(110.0))
                                                .text_xs()
                                                .number_of_lines(1)
                                                .text_color(move || theme().muted_foreground),
                                            )
                                            .child(
                                                text(move || {
                                                    filtered_rows.with(|r| {
                                                        r.get(at.index())
                                                            .map_or_else(String::new, |item| {
                                                                item.size.clone()
                                                            })
                                                    })
                                                })
                                                .w(px(76.0))
                                                .text_xs()
                                                .font_family(font_mono_family())
                                                .tabular_nums()
                                                .text_right()
                                                .text_color(move || theme().foreground),
                                            )
                                    },
                                )
                                .element_ref(&list_ref)
                                .flex_1()
                                .min_h(px(0.0))
                                .estimated_item_size(px(32.0))
                                .keys(move || {
                                    filtered_rows.with(|r| {
                                        r.iter()
                                            .map(LocalContainerImage::reference)
                                            .collect::<Vec<String>>()
                                    })
                                }),
                            )
                            .into_element()
                    }
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
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .flex_shrink_0()
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Root, Size};
    use nsh_testing::Headless;

    use super::*;
    use crate::domain::DivingService;
    use crate::ui::app_tokens;

    #[component]
    fn LocalPickerHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        let images: Vec<LocalContainerImage> = (0..90)
            .map(|i| LocalContainerImage {
                runtime: LocalRuntime::Docker,
                repository: format!("repo/service-{i:02}"),
                tag: "latest".to_owned(),
                id: format!("abc{i:09}"),
                created: "2 days ago".to_owned(),
                size: format!("{} MB", i + 10),
            })
            .collect();
        state.local_images.set(images);
        state
            .local_selected_ref
            .set(Some("docker://repo/service-00:latest".to_owned()));
        state.local_loading.set(false);
        state.local_error.set(None);
        state.local_picker_open.set(true);

        local_picker_dialog(state)
    }

    #[test]
    fn local_picker_filters_clears_hidden_selection_and_navigates_with_keys() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(Root::of::<LocalPickerHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start local picker harness");
        app.wait_for_within("repo/service-00:latest", Duration::from_secs(3))
            .expect("first local image visible");

        // Scroll past row 80 before filtering to verify virtual_list cell recycling & click accuracy.
        app.scroll("repo/service-00:latest", px(2600.0))
            .expect("scroll local picker list");
        app.wait_for_within("repo/service-85:latest", Duration::from_secs(3))
            .expect("recycled local picker row 85 rendered");
        app.press("repo/service-85:latest")
            .expect("select recycled row 85");

        // Filter hides service-85 when searching for service-0, clearing selection.
        app.type_text("Filter local container images", "service-0")
            .expect("filter to service-0x");
        app.wait_for_within("repo/service-00:latest", Duration::from_secs(3))
            .expect("filtered list shows service-00");
        assert!(!app.shows("repo/service-85:latest"));
        assert_eq!(app.screen().enabled("Analyze selected"), Ok(false));

        app.press_key("down")
            .expect("arrow down selects first filtered match");
        app.wait_until(
            "selection enabled after arrow down",
            Duration::from_secs(2),
            |s| s.enabled("Analyze selected") == Ok(true),
        )
        .expect("Analyze selected enabled after arrow down");
        app.press_key("escape")
            .expect("first Esc clears non-empty filter");
        app.wait_for_within("repo/service-00:latest", Duration::from_secs(3))
            .expect("clearing filter restores all rows");
        app.press_key("escape")
            .expect("second Esc on empty filter closes Local Images dialog");
        app.wait_for_gone("Analyze selected", Duration::from_secs(3))
            .expect("Local Images dialog closed");
    }
}
