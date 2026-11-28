//! Local Docker / Podman / Apple Container image picker dialog with daemon failure recovery.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use diving_core::{LocalContainerImage, LocalRuntime};
use nsh_core::AccessibilityState;
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::domain::{
    AppState, LocalImageSort, LocalImageSortField, SharedRef, SortDirection, sort_local_images,
};
use crate::i18n::{format_user_error, t};
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, SegmentOption, UiSize, font_mono_family,
    radius_inset, surface_active, theme, ui_badge, ui_banner, ui_button, ui_conditional_tooltip,
    ui_context_menu, ui_dialog, ui_empty_state, ui_icon_button, ui_menu_item, ui_menu_separator,
    ui_search_input_with_escape, ui_segmented, ui_skeleton, ui_spinner, ui_table_sort_header,
};

fn runtime_label_key(runtime: LocalRuntime) -> &'static str {
    match runtime {
        LocalRuntime::Docker => "local.docker",
        LocalRuntime::Podman => "local.podman",
        LocalRuntime::Container => "local.container",
    }
}

fn switch_runtime_label_key(runtime: LocalRuntime) -> &'static str {
    match runtime {
        LocalRuntime::Docker => "local.switch_docker",
        LocalRuntime::Podman => "local.switch_podman",
        LocalRuntime::Container => "local.switch_container",
    }
}

fn other_runtimes(current: LocalRuntime, order: [LocalRuntime; 3]) -> [LocalRuntime; 2] {
    let mut iter = order.into_iter().filter(|&rt| rt != current);
    let first = iter.next().unwrap_or(LocalRuntime::Docker);
    let second = iter.next().unwrap_or(LocalRuntime::Podman);
    [first, second]
}

fn column_sort_asc(sort: Option<LocalImageSort>, field: LocalImageSortField) -> Option<bool> {
    let active = sort?;
    (active.field == field).then_some(active.dir == SortDirection::Asc)
}

fn repo_column_available_width(viewport_width: Points) -> Points {
    let dialog_w = (viewport_width - px(48.0)).min(px(640.0));
    (dialog_w - px(426.0)).max(px(40.0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerViewState {
    Loading,
    Error,
    Empty,
    List,
}

/// Modal centered dialog listing images stored in local Docker, Podman, or Apple Container engines.
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
    let list_focused = signal(false);
    list_ref.on_focus_in(move || {
        let _ = list_focused.try_set(true);
    });
    list_ref.on_focus_out(move || {
        let _ = list_focused.try_set(false);
    });
    let focus_alive = Arc::new(AtomicBool::new(true));
    let focus_cleanup_alive = focus_alive.clone();
    cleanup(move || {
        focus_cleanup_alive.store(false, Ordering::SeqCst);
    });

    let focus_target = filter_ref;
    set_timeout(Duration::from_millis(20), move || {
        if focus_alive.load(Ordering::SeqCst) {
            focus_target.focus();
        }
    });

    let filter_sig = state.local_filter;
    let sort_sig = state.local_sort;
    let images_sig = state.local_images;
    let loading_sig = state.local_loading;
    let error_sig = state.local_error;
    let action_err_sig = state.local_action_error;
    let deleting_sig = state.local_deleting_ref;
    let runtime_order_sig = state.local_runtime_order;
    let local_runtime_sig = state.local_runtime;
    let selected_ref = state.local_selected_ref;

    let filtered_rows: Memo<SharedRef<Vec<LocalContainerImage>>> = computed(move || {
        let keyword = filter_sig.get().trim().to_ascii_lowercase();
        let sort = sort_sig.get();
        let mut filtered: Vec<LocalContainerImage> = images_sig
            .get()
            .into_iter()
            .filter(|img| {
                keyword.is_empty()
                    || img.repository.to_ascii_lowercase().contains(&keyword)
                    || img.tag.to_ascii_lowercase().contains(&keyword)
                    || img.id.to_ascii_lowercase().contains(&keyword)
            })
            .collect();
        sort_local_images(&mut filtered, sort);
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

    let active_state = state.clone();
    let list_active = computed(move || {
        list_focused.get()
            && active_state.local_picker_open.get()
            && !active_state.command_palette_open.get()
    });
    let act_state = state.clone();
    effect(move || {
        if !list_active.get() {
            return;
        }
        let act_opts = KeyOptions {
            when: When::NotEditing,
            repeat: false,
            for_focus: true,
            ..Default::default()
        };
        let enter_state = act_state.clone();
        on_key_with("enter", act_opts, move |_| {
            enter_state.analyze_selected_local_image();
        });
        let space_state = act_state.clone();
        on_key_with("space", act_opts, move |_| {
            space_state.analyze_selected_local_image();
        });
        let backspace_state = act_state.clone();
        on_key_with("backspace", act_opts, move |_| {
            backspace_state.request_delete_selected_local_image();
        });
        let delete_state = act_state.clone();
        on_key_with("delete", act_opts, move |_| {
            delete_state.request_delete_selected_local_image();
        });
    });

    let dismiss_err_state = state.clone();
    let list_state = state;

    view()
        .flex_col()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(dynamic(move || {
                    let order = runtime_order_sig.get();
                    let change_state = rt_state.clone();
                    ui_segmented(
                        order
                            .into_iter()
                            .map(|rt| {
                                let key = runtime_label_key(rt);
                                SegmentOption::new(rt, move || {
                                    SharedString::from(t(lang.get(), key))
                                })
                            })
                            .collect(),
                        local_runtime_sig,
                        UiSize::Sm,
                        "Container runtime",
                        move |next| change_state.set_local_runtime(next),
                    )
                }))
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
        .child(when(
            move || action_err_sig.get().is_some(),
            move || {
                let dismiss_state = dismiss_err_state.clone();
                ui_banner(
                    BannerTone::Danger,
                    move || {
                        let raw = action_err_sig.get().unwrap_or_default();
                        SharedString::from(format_user_error(lang.get(), &raw))
                    },
                    Some(
                        ui_icon_button(
                            IconName::Close,
                            "Dismiss error",
                            Some(move || SharedString::from(t(lang.get(), "common.close"))),
                            ButtonVariant::Ghost,
                            UiSize::Xs,
                            false,
                            false,
                            move || dismiss_state.dismiss_local_action_error(),
                        )
                        .into_element(),
                    ),
                )
            },
        ))
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
                        let current_rt = list_state.local_runtime.get();
                        let rt_order = runtime_order_sig.get();
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
                                                .children(
                                                    other_runtimes(current_rt, rt_order)
                                                        .into_iter()
                                                        .map(|alt| {
                                                            let switch_state = list_state.clone();
                                                            let key = switch_runtime_label_key(alt);
                                                            ui_button(
                                                                move || {
                                                                    SharedString::from(t(
                                                                        lang.get(),
                                                                        key,
                                                                    ))
                                                                },
                                                                ButtonVariant::Outline,
                                                                UiSize::Sm,
                                                                None,
                                                                false,
                                                                false,
                                                                move || {
                                                                    switch_state
                                                                        .set_local_runtime(alt)
                                                                },
                                                            )
                                                        }),
                                                )
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
                        let repo_sort_state = list_state.clone();
                        let id_sort_state = list_state.clone();
                        let created_sort_state = list_state.clone();
                        let size_sort_state = list_state.clone();
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
                                    .child(view().flex_1().min_w(px(0.0)).h_full().child(
                                        ui_table_sort_header(
                                            move || {
                                                SharedString::from(t(lang.get(), "local.col_repo"))
                                            },
                                            move || {
                                                column_sort_asc(
                                                    sort_sig.get(),
                                                    LocalImageSortField::Repository,
                                                )
                                            },
                                            false,
                                            move || {
                                                repo_sort_state.toggle_local_sort(
                                                    LocalImageSortField::Repository,
                                                )
                                            },
                                        ),
                                    ))
                                    .child(view().w(px(108.0)).h_full().flex_shrink_0().child(
                                        ui_table_sort_header(
                                            move || {
                                                SharedString::from(t(lang.get(), "local.col_id"))
                                            },
                                            move || {
                                                column_sort_asc(
                                                    sort_sig.get(),
                                                    LocalImageSortField::Id,
                                                )
                                            },
                                            false,
                                            move || {
                                                id_sort_state
                                                    .toggle_local_sort(LocalImageSortField::Id)
                                            },
                                        ),
                                    ))
                                    .child(view().w(px(114.0)).h_full().flex_shrink_0().child(
                                        ui_table_sort_header(
                                            move || {
                                                SharedString::from(t(
                                                    lang.get(),
                                                    "local.col_created",
                                                ))
                                            },
                                            move || {
                                                column_sort_asc(
                                                    sort_sig.get(),
                                                    LocalImageSortField::Created,
                                                )
                                            },
                                            false,
                                            move || {
                                                created_sort_state
                                                    .toggle_local_sort(LocalImageSortField::Created)
                                            },
                                        ),
                                    ))
                                    .child(view().w(px(78.0)).h_full().flex_shrink_0().child(
                                        ui_table_sort_header(
                                            move || {
                                                SharedString::from(t(lang.get(), "local.col_size"))
                                            },
                                            move || {
                                                column_sort_asc(
                                                    sort_sig.get(),
                                                    LocalImageSortField::Size,
                                                )
                                            },
                                            true,
                                            move || {
                                                size_sort_state
                                                    .toggle_local_sort(LocalImageSortField::Size)
                                            },
                                        ),
                                    ))
                                    .child(view().w(px(26.0)).flex_shrink_0()),
                            )
                            .child(
                                virtual_list(
                                    move || filtered_rows.with(|r| r.len()),
                                    move |at| {
                                        let click_state = row_state.clone();
                                        let focus_state = row_state.clone();
                                        let key_state = row_state.clone();
                                        let trash_state = row_state.clone();
                                        let ctx_analyze_state = row_state.clone();
                                        let ctx_delete_state = row_state.clone();

                                        let row_main = view()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .h_full()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .role(AccessibilityRole::Button)
                                            .focusable(true)
                                            .accessibility_state(move || {
                                                let row_ref = filtered_rows.with(|r| {
                                                    r.get(at.index())
                                                        .map(LocalContainerImage::reference)
                                                });
                                                let is_selected = row_ref.is_some()
                                                    && row_ref == selected_ref.get();
                                                AccessibilityState {
                                                    selected: is_selected,
                                                    ..AccessibilityState::default()
                                                }
                                            })
                                            .focus_visible(|s| {
                                                s.outline(px(2.0))
                                                    .outline_color(move || theme().ring)
                                            })
                                            .on_focus(move || {
                                                if let Some(reference) = filtered_rows
                                                    .with_untracked(|r| {
                                                        r.get(at.index())
                                                            .map(LocalContainerImage::reference)
                                                    })
                                                {
                                                    focus_state
                                                        .local_selected_ref
                                                        .set(Some(reference));
                                                }
                                            })
                                            .on_key_down(move |ev: &KeyEvent| {
                                                if matches!(
                                                    ev.key.as_str(),
                                                    "Enter" | "Space" | " "
                                                ) {
                                                    if key_state
                                                        .local_selected_ref
                                                        .get_untracked()
                                                        .is_none()
                                                        && let Some(reference) =
                                                            filtered_rows.with(|r| {
                                                                r.get(at.index()).map(
                                                                    LocalContainerImage::reference,
                                                                )
                                                            })
                                                    {
                                                        key_state
                                                            .local_selected_ref
                                                            .set(Some(reference));
                                                    }
                                                    key_state.analyze_selected_local_image();
                                                } else if matches!(
                                                    ev.key.as_str(),
                                                    "Backspace" | "Delete"
                                                ) {
                                                    if key_state
                                                        .local_selected_ref
                                                        .get_untracked()
                                                        .is_none()
                                                        && let Some(reference) =
                                                            filtered_rows.with(|r| {
                                                                r.get(at.index()).map(
                                                                    LocalContainerImage::reference,
                                                                )
                                                            })
                                                    {
                                                        key_state
                                                            .local_selected_ref
                                                            .set(Some(reference));
                                                    }
                                                    key_state.request_delete_selected_local_image();
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
                                                view()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .h_full()
                                                    .flex_row()
                                                    .items_center()
                                                    .tooltip(ui_conditional_tooltip(
                                                        move || {
                                                            SharedString::from(filtered_rows.with(
                                                                |r| {
                                                                    r.get(at.index()).map_or_else(
                                                                        String::new,
                                                                        LocalContainerImage::display_name,
                                                                    )
                                                                },
                                                            ))
                                                        },
                                                        move || {
                                                            let avail = repo_column_available_width(
                                                                window().width,
                                                            );
                                                            filtered_rows.with(|r| {
                                                                r.get(at.index()).is_some_and(
                                                                    |img| {
                                                                        let name =
                                                                            img.display_name();
                                                                        measure_text(
                                                                            "500 12px system-ui",
                                                                            &name,
                                                                        ) > avail
                                                                    },
                                                                )
                                                            })
                                                        },
                                                    ))
                                                    .child(
                                                        text(move || {
                                                            filtered_rows.with(|r| {
                                                                r.get(at.index()).map_or_else(
                                                                    String::new,
                                                                    LocalContainerImage::display_name,
                                                                )
                                                            })
                                                        })
                                                        .w_full()
                                                        .text_xs()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .number_of_lines(1)
                                                        .text_color(move || theme().foreground),
                                                    ),
                                            )
                                            .child(
                                                view()
                                                    .w(px(108.0))
                                                    .flex_shrink_0()
                                                    .items_start()
                                                    .child(ui_badge(
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
                                                    )),
                                            )
                                            .child(
                                                text(move || {
                                                    filtered_rows.with(|r| {
                                                        r.get(at.index())
                                                            .map_or_else(String::new, |item| {
                                                                item.created.clone()
                                                            })
                                                    })
                                                })
                                                .w(px(114.0))
                                                .flex_shrink_0()
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
                                                .w(px(78.0))
                                                .flex_shrink_0()
                                                .text_xs()
                                                .font_family(font_mono_family())
                                                .tabular_nums()
                                                .text_right()
                                                .text_color(move || theme().foreground),
                                            );

                                        let trailing_action = view()
                                            .w(px(26.0))
                                            .flex_shrink_0()
                                            .items_center()
                                            .justify_center()
                                            .child(dynamic(move || {
                                                let row_ref = filtered_rows.with(|r| {
                                                    r.get(at.index())
                                                        .map(LocalContainerImage::reference)
                                                });
                                                let is_this_deleting = row_ref.is_some()
                                                    && row_ref.as_deref()
                                                        == deleting_sig.get().as_deref();
                                                if is_this_deleting {
                                                    ui_spinner(px(14.0), move || {
                                                        theme().muted_foreground
                                                    })
                                                    .into_element()
                                                } else {
                                                    let row_trash_state = trash_state.clone();
                                                    ui_icon_button(
                                                        IconName::Trash,
                                                        "Delete local image",
                                                        None::<SharedString>,
                                                        ButtonVariant::Ghost,
                                                        UiSize::Xs,
                                                        false,
                                                        move || deleting_sig.get().is_some(),
                                                        move || {
                                                            if let Some(img) =
                                                                filtered_rows.with(|r| {
                                                                    r.get(at.index()).cloned()
                                                                })
                                                            {
                                                                row_trash_state
                                                                    .request_delete_local_image(
                                                                        img,
                                                                    );
                                                            }
                                                        },
                                                    )
                                                    .into_element()
                                                }
                                            }));

                                        let row_view = view()
                                            .w_full()
                                            .h(px(32.0))
                                            .px_3()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
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
                                            .child(row_main)
                                            .child(trailing_action);

                                        ui_context_menu(row_view, |_| {})
                                            .child(ui_menu_item(
                                                move || {
                                                    SharedString::from(t(
                                                        lang.get(),
                                                        "local.ctx_analyze",
                                                    ))
                                                },
                                                Some(IconName::Search),
                                                None,
                                                move || deleting_sig.get().is_some(),
                                                move || {
                                                    if let Some(reference) =
                                                        filtered_rows.with(|r| {
                                                            r.get(at.index())
                                                                .map(LocalContainerImage::reference)
                                                        })
                                                    {
                                                        ctx_analyze_state
                                                            .local_picker_open
                                                            .set(false);
                                                        ctx_analyze_state.analyze(reference, None);
                                                    }
                                                },
                                            ))
                                            .child(ui_menu_item(
                                                move || {
                                                    SharedString::from(t(
                                                        lang.get(),
                                                        "local.ctx_copy_ref",
                                                    ))
                                                },
                                                Some(IconName::Copy),
                                                None,
                                                false,
                                                move || {
                                                    if let Some(reference) =
                                                        filtered_rows.with(|r| {
                                                            r.get(at.index())
                                                                .map(LocalContainerImage::reference)
                                                        })
                                                    {
                                                        drop(set_clipboard(reference));
                                                    }
                                                },
                                            ))
                                            .child(ui_menu_separator())
                                            .child(ui_menu_item(
                                                move || {
                                                    SharedString::from(t(
                                                        lang.get(),
                                                        "local.ctx_delete",
                                                    ))
                                                },
                                                Some(IconName::Trash),
                                                None,
                                                move || deleting_sig.get().is_some(),
                                                move || {
                                                    if let Some(img) = filtered_rows
                                                        .with(|r| r.get(at.index()).cloned())
                                                    {
                                                        ctx_delete_state
                                                            .request_delete_local_image(img);
                                                    }
                                                },
                                            ))
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
    let delete_state = state.clone();
    let analyze_state = state.clone();
    let sel_sig = state.local_selected_ref;
    let del_sig = state.local_deleting_ref;

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
                    move || SharedString::from(t(lang.get(), "local.delete_btn")),
                    ButtonVariant::Outline,
                    UiSize::Md,
                    Some(IconName::Trash),
                    move || sel_sig.get().is_none() || del_sig.get().is_some(),
                    false,
                    move || delete_state.request_delete_selected_local_image(),
                ))
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
                    move || sel_sig.get().is_none() || del_sig.get().is_some(),
                    false,
                    move || analyze_state.analyze_selected_local_image(),
                )),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, SystemTime};

    use nsh_engine::{Clock, EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{DialogAnswer, Headless, Screen, windowed_root};

    use super::*;
    use crate::domain::DivingService;
    use crate::ui::app_tokens;

    fn item_origin(screen: &Screen, selector: &str) -> Option<(Points, Points)> {
        screen
            .layout(selector)
            .ok()
            .flatten()
            .map(|layout| (layout.bounds.origin.x, layout.bounds.origin.y))
    }

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
                size_bytes: None,
                created_at: None,
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
            ..EngineConfig::new(windowed_root::<LocalPickerHarness>().tokens(app_tokens()))
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

    #[component]
    fn LocalPickerKeyboardActivationHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        let images: Vec<LocalContainerImage> = (0..5)
            .map(|i| LocalContainerImage {
                runtime: LocalRuntime::Docker,
                repository: format!("repo/item-{i:02}"),
                tag: "latest".to_owned(),
                id: format!("def{i:09}"),
                created: "today".to_owned(),
                size: "12 MB".to_owned(),
                size_bytes: None,
                created_at: None,
            })
            .collect();
        state.local_images.set(images);
        state
            .local_selected_ref
            .set(Some("docker://repo/item-02:latest".to_owned()));
        state.local_loading.set(false);
        state.local_error.set(None);
        state.local_picker_open.set(true);

        let pending = state.pending_image;
        let image_input = state.image_input;
        let sel = state.local_selected_ref;
        view()
            .flex_col()
            .child(text(move || {
                format!(
                    "sel={};analyzed={}",
                    sel.get().unwrap_or_default(),
                    pending.get().unwrap_or_else(|| image_input.get())
                )
            }))
            .child(local_picker_dialog(state))
    }

    #[test]
    fn local_picker_arrow_plus_space_or_enter_acts_on_active_selection_and_sets_selected_state() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(
                windowed_root::<LocalPickerKeyboardActivationHarness>().tokens(app_tokens()),
            )
        };
        let mut app = Headless::start(config).expect("start local picker keyboard harness");
        app.wait_for_within("repo/item-00:latest", Duration::from_secs(3))
            .expect("rows visible");
        thread::sleep(Duration::from_millis(35));

        // Tab until focus enters row 0 (repo/item-00:latest), overriding item-02 via row on_focus.
        for _ in 0..24 {
            app.press_key("tab").expect("tab forward");
            if app
                .screen()
                .selected("role:button[/repo\\/item-00:latest/]")
                == Ok(true)
            {
                break;
            }
        }
        assert_eq!(
            app.screen()
                .selected("role:button[/repo\\/item-00:latest/]"),
            Ok(true)
        );

        // Press Down so active selection moves to item-01 while focus stays on row 0, then Space activates item-01.
        app.press_key("down").expect("arrow down to item-01");
        app.wait_until(
            "active selection moved to item-01",
            Duration::from_secs(2),
            |s| s.selected("role:button[/repo\\/item-01:latest/]") == Ok(true),
        )
        .expect("active selection moved to item-01");
        assert_eq!(
            app.screen()
                .selected("role:button[/repo\\/item-00:latest/]"),
            Ok(false)
        );
        assert_eq!(
            app.screen()
                .selected("role:button[/repo\\/item-01:latest/]"),
            Ok(true)
        );

        app.press_key("space")
            .expect("space activates active item-01 rather than stale focused row 0");
        app.wait_for_within(
            "sel=docker://repo/item-01:latest;analyzed=docker://repo/item-01:latest",
            Duration::from_secs(2),
        )
        .expect("Space analyzed active item-01");
    }

    #[component]
    fn LocalPickerContainerSwitchHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.local_runtime.set(LocalRuntime::Docker);
        state.local_images.set(Vec::new());
        state.local_loading.set(false);
        state
            .local_error
            .set(Some("docker images failed: daemon not running".to_owned()));
        state.local_picker_open.set(true);

        let reset_state = state.clone();
        let rt_sig = state.local_runtime;
        view()
            .flex_col()
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(text(move || format!("rt={:?}", rt_sig.get())))
                    .child(ui_button(
                        "ResetDockerError",
                        ButtonVariant::Ghost,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            reset_state.local_request_id.set(99);
                            reset_state.local_runtime.set(LocalRuntime::Docker);
                            reset_state.local_loading.set(false);
                            reset_state
                                .local_error
                                .set(Some("docker images failed: daemon not running".to_owned()));
                        },
                    )),
            )
            .child(local_picker_dialog(state))
    }

    #[test]
    fn local_picker_supports_container_segment_and_error_recovery_switch() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(
                windowed_root::<LocalPickerContainerSwitchHarness>().tokens(app_tokens()),
            )
        };
        let mut app = Headless::start(config).expect("start container switch harness");
        app.wait_for_within("Switch to Container", Duration::from_secs(3))
            .expect("Switch to Container recovery button visible");
        assert!(app.shows("Switch to Podman"));
        assert!(app.shows("rt=Docker"));

        app.press("Switch to Container")
            .expect("switch runtime via error recovery button");
        app.wait_for_within("rt=Container", Duration::from_secs(3))
            .expect("runtime switched to Container from recovery button");

        app.press("ResetDockerError")
            .expect("reset back to Docker error");
        app.wait_for_within("rt=Docker", Duration::from_secs(3))
            .expect("reset to Docker");
        app.press("Container")
            .expect("select Container segment option");
        app.wait_for_within("rt=Container", Duration::from_secs(3))
            .expect("runtime switched to Container from segmented control");
    }

    #[component]
    fn LocalPickerSortHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.local_runtime_order.set([
            LocalRuntime::Container,
            LocalRuntime::Docker,
            LocalRuntime::Podman,
        ]);
        state.local_runtime.set(LocalRuntime::Container);
        state.local_images.set(vec![
            LocalContainerImage {
                runtime: LocalRuntime::Container,
                repository: "alpha/small".to_owned(),
                tag: "1.0".to_owned(),
                id: "111111111111".to_owned(),
                created: "2026-07-01 10:00".to_owned(),
                size: "45.0 MiB".to_owned(),
                size_bytes: None,
                created_at: None,
            },
            LocalContainerImage {
                runtime: LocalRuntime::Container,
                repository: "zeta/big".to_owned(),
                tag: "2.0".to_owned(),
                id: "222222222222".to_owned(),
                created: "2026-10-01 10:00".to_owned(),
                size: "250.0 MiB".to_owned(),
                size_bytes: None,
                created_at: None,
            },
            LocalContainerImage {
                runtime: LocalRuntime::Container,
                repository: "beta/mid".to_owned(),
                tag: "1.5".to_owned(),
                id: "333333333333".to_owned(),
                created: "2026-08-15 10:00".to_owned(),
                size: "120.0 MiB".to_owned(),
                size_bytes: None,
                created_at: None,
            },
        ]);
        state
            .local_selected_ref
            .set(Some("container://beta/mid:1.5".to_owned()));
        state.local_loading.set(false);
        state.local_error.set(None);
        state.local_picker_open.set(true);

        let reorder_state = state.clone();
        let sel_sig = state.local_selected_ref;
        view()
            .flex_col()
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(text(move || {
                        format!("sel={}", sel_sig.get().unwrap_or_default())
                    }))
                    .child(ui_button(
                        "ReorderPodmanFirst",
                        ButtonVariant::Ghost,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            reorder_state.local_runtime_order.set([
                                LocalRuntime::Podman,
                                LocalRuntime::Container,
                                LocalRuntime::Docker,
                            ]);
                        },
                    )),
            )
            .child(local_picker_dialog(state))
    }

    #[test]
    fn local_picker_sorts_columns_and_orders_installed_runtimes() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<LocalPickerSortHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start local picker sort harness");
        app.wait_for_within("alpha/small:1.0", Duration::from_secs(3))
            .expect("initial rows rendered");

        // Verify installed-first segment horizontal order: Container -> Docker -> Podman.
        let container_x = item_origin(app.screen(), "Container")
            .expect("Container origin")
            .0;
        let docker_x = item_origin(app.screen(), "Docker")
            .expect("Docker origin")
            .0;
        let podman_x = item_origin(app.screen(), "Podman")
            .expect("Podman origin")
            .0;
        assert!(container_x < docker_x);
        assert!(docker_x < podman_x);

        // Verify dynamic order updates cleanly: Podman -> Container -> Docker.
        app.press("ReorderPodmanFirst")
            .expect("reorder Podman first");
        app.wait_until(
            "Podman segment moved before Container",
            Duration::from_secs(2),
            |s| {
                matches!(
                    (
                        item_origin(s, "Podman"),
                        item_origin(s, "Container"),
                        item_origin(s, "Docker"),
                    ),
                    (Some((px, _)), Some((cx, _)), Some((dx, _))) if px < cx && cx < dx
                )
            },
        )
        .expect("segmented control updated order");

        // Initial unsorted order: alpha -> zeta -> beta.
        let alpha_y0 = item_origin(app.screen(), "alpha/small:1.0")
            .expect("alpha y0")
            .1;
        let zeta_y0 = item_origin(app.screen(), "zeta/big:2.0")
            .expect("zeta y0")
            .1;
        assert!(alpha_y0 < zeta_y0);

        // 1st click on Size -> descending (250 MiB > 120 MiB > 45 MiB), keeping selected ref.
        app.press("Size").expect("sort Size descending");
        app.wait_for_within("↓", Duration::from_secs(2))
            .expect("descending arrow visible");
        app.wait_until("Size descending row order", Duration::from_secs(2), |s| {
            matches!(
                (
                    item_origin(s, "zeta/big:2.0"),
                    item_origin(s, "beta/mid:1.5"),
                    item_origin(s, "alpha/small:1.0"),
                ),
                (Some((_, zy)), Some((_, by)), Some((_, ay))) if zy < by && by < ay
            )
        })
        .expect("rows sorted largest first");
        assert!(app.shows("sel=container://beta/mid:1.5"));

        // 2nd click on Size -> ascending (45 MiB < 120 MiB < 250 MiB).
        app.press("Size").expect("sort Size ascending");
        app.wait_for_within("↑", Duration::from_secs(2))
            .expect("ascending arrow visible");
        app.wait_until("Size ascending row order", Duration::from_secs(2), |s| {
            matches!(
                (
                    item_origin(s, "alpha/small:1.0"),
                    item_origin(s, "beta/mid:1.5"),
                    item_origin(s, "zeta/big:2.0"),
                ),
                (Some((_, ay)), Some((_, by)), Some((_, zy))) if ay < by && by < zy
            )
        })
        .expect("rows sorted smallest first");
        assert!(app.shows("sel=container://beta/mid:1.5"));

        // 3rd click on Size -> clears sort back to original CLI order.
        app.press("Size").expect("clear Size sort");
        app.wait_for_gone("↑", Duration::from_secs(2))
            .expect("sort indicator cleared");
        app.wait_until(
            "unsorted initial row order restored",
            Duration::from_secs(2),
            |s| {
                matches!(
                    (
                        item_origin(s, "alpha/small:1.0"),
                        item_origin(s, "zeta/big:2.0"),
                        item_origin(s, "beta/mid:1.5"),
                    ),
                    (Some((_, ay)), Some((_, zy)), Some((_, by))) if ay < zy && zy < by
                )
            },
        )
        .expect("initial CLI order restored");

        // Created column sorts newest first by parsed timestamp.
        app.press("Created").expect("sort Created descending");
        app.wait_for_within("↓", Duration::from_secs(2))
            .expect("Created descending arrow visible");
        app.wait_until(
            "Created descending newest first",
            Duration::from_secs(2),
            |s| {
                matches!(
                    (
                        item_origin(s, "zeta/big:2.0"),
                        item_origin(s, "beta/mid:1.5"),
                        item_origin(s, "alpha/small:1.0"),
                    ),
                    (Some((_, zy)), Some((_, by)), Some((_, ay))) if zy < by && by < ay
                )
            },
        )
        .expect("Created column sorted newest first");
    }

    #[component]
    fn LocalPickerDeleteHarness() -> impl IntoElement {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let hook_log = Arc::clone(&log);
        let service = DivingService::new().with_local_remove_hook(move |rt, target, id, force| {
            if let Ok(mut entries) = hook_log.lock() {
                entries.push(format!("{rt:?}:{target}:{id}:force={force}"));
            }
            if target == "in-use:2.0" && !force {
                return Err("docker image rm in-use:2.0 failed: conflict: unable to remove repository reference \"in-use:2.0\" (must force) - container web-1 is using its referenced image".to_owned());
            }
            Ok(())
        });

        let state = AppState::new(service);
        state.local_runtime_order.set([
            LocalRuntime::Docker,
            LocalRuntime::Podman,
            LocalRuntime::Container,
        ]);
        state.local_runtime.set(LocalRuntime::Docker);
        state.local_images.set(vec![
            LocalContainerImage {
                runtime: LocalRuntime::Docker,
                repository: "unused".to_owned(),
                tag: "1.0".to_owned(),
                id: "111111111111".to_owned(),
                created: "today".to_owned(),
                size: "12 MiB".to_owned(),
                size_bytes: Some(12_582_912),
                created_at: None,
            },
            LocalContainerImage {
                runtime: LocalRuntime::Docker,
                repository: "in-use".to_owned(),
                tag: "2.0".to_owned(),
                id: "222222222222".to_owned(),
                created: "today".to_owned(),
                size: "34 MiB".to_owned(),
                size_bytes: Some(35_651_584),
                created_at: None,
            },
        ]);
        state
            .local_selected_ref
            .set(Some("docker://unused:1.0".to_owned()));
        state.local_loading.set(false);
        state.local_error.set(None);
        state.local_picker_open.set(true);

        let sel_sig = state.local_selected_ref;
        let del_sig = state.local_deleting_ref;
        let imgs_sig = state.local_images;
        let calls_sig = signal(String::new());
        effect(move || {
            let _ = del_sig.get();
            let _ = imgs_sig.get();
            if let Ok(entries) = log.lock() {
                calls_sig.set(entries.join("|"));
            }
        });

        view()
            .flex_col()
            .child(text(move || {
                format!(
                    "sel={};calls={}",
                    sel_sig.get().unwrap_or_default(),
                    calls_sig.get()
                )
            }))
            .child(local_picker_dialog(state))
    }

    #[test]
    fn local_picker_deletes_single_image_and_supports_force_delete_on_in_use() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<LocalPickerDeleteHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start local picker delete harness");
        app.answer_dialogs_myself();
        app.wait_for_within("text:unused:1.0", Duration::from_secs(3))
            .expect("initial images rendered");
        assert!(app.shows("sel=docker://unused:1.0;calls="));

        // 1. Cancel confirmation keeps the image and runs no CLI command.
        app.press("Delete…").expect("click footer Delete… button");
        app.wait_until("delete confirm dialog shown", Duration::from_secs(2), |s| {
            s.dialogs().len() == 1
        })
        .expect("first confirmation dialog appeared");
        assert_eq!(
            app.pending_dialog().and_then(|d| d.title.as_deref()),
            Some("Delete “unused:1.0”?")
        );
        app.answer_dialog(DialogAnswer::Press("Cancel".to_owned()))
            .expect("cancel image delete");
        assert!(app.shows("unused:1.0"));
        assert!(app.shows("sel=docker://unused:1.0;calls="));

        // 2. Confirm normal delete removes unused:1.0 and advances selection to neighbor in-use:2.0.
        app.press("Delete…").expect("click Delete… again");
        app.wait_until(
            "second delete confirm dialog shown",
            Duration::from_secs(2),
            |s| s.dialogs().len() == 2,
        )
        .expect("second confirmation dialog appeared");
        app.answer_dialog(DialogAnswer::Press("Delete".to_owned()))
            .expect("confirm normal delete");
        app.wait_for_gone("text:unused:1.0", Duration::from_secs(3))
            .expect("unused:1.0 deleted");
        app.wait_for_within(
            "sel=docker://in-use:2.0;calls=Docker:unused:1.0:111111111111:force=false",
            Duration::from_secs(3),
        )
        .expect("neighbor selected and normal remove recorded");

        // 3. In-use image triggers secondary Force delete confirmation and succeeds with --force.
        app.press("Delete…")
            .expect("click Delete… on selected in-use:2.0");
        app.wait_until(
            "in-use normal confirmation dialog shown",
            Duration::from_secs(2),
            |s| s.dialogs().len() == 3,
        )
        .expect("third confirmation dialog appeared");
        assert_eq!(
            app.pending_dialog().and_then(|d| d.title.as_deref()),
            Some("Delete “in-use:2.0”?")
        );
        app.answer_dialog(DialogAnswer::Press("Delete".to_owned()))
            .expect("attempt non-force delete on in-use:2.0");

        app.wait_until(
            "force delete confirmation dialog shown",
            Duration::from_secs(3),
            |s| s.dialogs().len() == 4,
        )
        .expect("force delete confirmation dialog appeared");
        assert_eq!(
            app.pending_dialog().and_then(|d| d.title.as_deref()),
            Some("Force delete “in-use:2.0”?")
        );
        app.answer_dialog(DialogAnswer::Press("Force delete".to_owned()))
            .expect("confirm force delete");

        app.wait_for_gone("text:in-use:2.0", Duration::from_secs(3))
            .expect("in-use:2.0 removed after force delete");
        app.wait_for_within(
            "sel=;calls=Docker:unused:1.0:111111111111:force=false|Docker:in-use:2.0:222222222222:force=false|Docker:in-use:2.0:222222222222:force=true",
            Duration::from_secs(3),
        )
        .expect("force delete call recorded");
    }

    #[component]
    fn LocalPickerTooltipHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.local_runtime.set(LocalRuntime::Container);
        state.local_images.set(vec![
            LocalContainerImage {
                runtime: LocalRuntime::Container,
                repository: "docker.dragonflydb.io/dragonflydb/dragonfly".to_owned(),
                tag: "v1.26.0".to_owned(),
                id: "061f00b201b5".to_owned(),
                created: "2026-10-07 21:48".to_owned(),
                size: "46.4 MiB".to_owned(),
                size_bytes: Some(48_653_926),
                created_at: None,
            },
            LocalContainerImage {
                runtime: LocalRuntime::Container,
                repository: "redis".to_owned(),
                tag: "7".to_owned(),
                id: "71da9275c5f3".to_owned(),
                created: "2026-08-25 00:46".to_owned(),
                size: "41.5 MiB".to_owned(),
                size_bytes: Some(43_515_904),
                created_at: None,
            },
        ]);
        state.local_loading.set(false);
        state.local_error.set(None);
        state.local_picker_open.set(true);

        local_picker_dialog(state)
    }

    #[test]
    fn local_picker_shows_hover_tooltip_only_for_truncated_repo_names() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            clock: Clock::manual(SystemTime::UNIX_EPOCH),
            ..EngineConfig::new(windowed_root::<LocalPickerTooltipHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start local picker tooltip harness");
        app.first_render().expect("initial render");
        app.wait_for_within("text:redis:7", Duration::from_secs(3))
            .expect("rows visible");

        let long_name = "docker.dragonflydb.io/dragonflydb/dragonfly:v1.26.0";
        let long_selector = format!("text:{long_name}");
        assert_eq!(app.screen().count(&long_selector), Ok(1));

        // Hovering a truncated Repository:Tag cell mounts the tooltip with the full reference.
        app.hover(&long_selector)
            .expect("hover truncated repository name");
        app.fast_forward(Duration::from_millis(600))
            .expect("advance past tooltip hover delay");
        app.wait_until(
            "full repository tooltip appears on hover",
            Duration::from_secs(2),
            |s| s.count(&long_selector) == Ok(2),
        )
        .expect("full repository tooltip shown");

        // Opening the row context menu dismisses/suppresses the tooltip so overlays do not stack.
        app.right_click("role:button[/dragonfly/]")
            .expect("right click row with tooltip open");
        app.wait_for("role:menuitem[Analyze image]")
            .expect("row context menu opened");
        app.wait_until(
            "tooltip hidden while context menu is open",
            Duration::from_secs(2),
            |s| s.count(&long_selector) == Ok(1),
        )
        .expect("tooltip hidden when context menu opens");
        app.press_key("escape").expect("dismiss context menu");
        app.wait_for_gone("role:menuitem[Analyze image]", Duration::from_secs(2))
            .expect("row context menu closed");

        // Hovering a short Repository:Tag name does not display a redundant tooltip.
        app.hover("text:redis:7").expect("hover short repo name");
        app.fast_forward(Duration::from_millis(600))
            .expect("wait past tooltip hover delay on short repo");
        assert_eq!(app.screen().count("text:redis:7"), Ok(1));
    }
}
