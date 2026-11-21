//! Tab 1 (`File Tree`): collapsible `CommandInspector`, filter toolbar, and `virtual_list` tree.

use std::sync::Arc;
use std::time::Duration;

use diving_core::DockerAnalyzeResult;
use nsh_core::prelude::*;

use crate::domain::{
    AppState, FileTreeRow, classify_layer_opcode, count_layer_file_ops, flatten_file_tree,
    format_bytes, format_command_lines,
};
use crate::i18n::{UiLang, t, tf};
use crate::ui::{
    BadgeTone, ButtonVariant, IconName, SelectItem, UiSize, font_mono_family, radius_inset,
    surface_active, text_tertiary, theme, ui_badge, ui_button, ui_checkbox, ui_context_menu,
    ui_empty_state, ui_icon, ui_icon_button, ui_menu_item, ui_opcode_pill, ui_search_input,
    ui_select, ui_size_bar, ui_status_badge,
};

fn min_size_items(lang: UiLang) -> Vec<SelectItem> {
    vec![
        SelectItem::new("0", t(lang, "tree.size_all")),
        SelectItem::new("102400", t(lang, "tree.size_100k")),
        SelectItem::new("1048576", t(lang, "tree.size_1m")),
        SelectItem::new("10485760", t(lang, "tree.size_10m")),
    ]
}

/// Renders `CommandInspector`, the filter toolbar, column header, and virtualized file tree.
pub fn file_tree_tab(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let selected_layer = state.selected_layer;
    let res_for_rows = res.clone();
    let filter_state = state.clone();

    let rows: Memo<Vec<FileTreeRow>> = computed(move || {
        let layer_idx = selected_layer.get();
        let Some(layer_items) = res_for_rows.file_tree_list.get(layer_idx) else {
            return Vec::new();
        };
        let filter = filter_state.current_tree_filter();
        let expanded = filter_state.tree_expanded.get();
        let collapsed = filter_state.tree_collapsed.get();
        flatten_file_tree(layer_items, &filter, &expanded, &collapsed)
    });

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_col()
        .child(command_inspector(state.clone(), res.clone()))
        .child(filter_toolbar(state.clone()))
        .child(column_header(state.clone()))
        .child(virtual_tree(state, res, rows))
}

fn command_inspector(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let lang = state.lang;
    let selected_layer = state.selected_layer;
    let command_expanded = state.command_expanded;
    let command_wrap = state.command_wrap;
    let copied = signal(false);

    view()
        .w_full()
        .flex_col()
        .bg(move || theme().card)
        .border_b_1()
        .border_color(move || theme().border)
        .child(dynamic(move || {
            let idx = selected_layer.get();
            let Some(layer) = res.layers.get(idx) else {
                return view().into_element();
            };
            let counts = res
                .file_tree_list
                .get(idx)
                .map_or_else(Default::default, |t| count_layer_file_ops(t));
            let (opcode, _) = classify_layer_opcode(&layer.cmd);
            let lines = format_command_lines(&layer.cmd);
            let total_lines = lines.len();
            let show_all = command_expanded.get() || total_lines <= 6;
            let visible_lines: Vec<String> = if show_all {
                lines
            } else {
                lines.into_iter().take(6).collect()
            };
            let hidden_count = total_lines.saturating_sub(6);
            let digest_to_copy = layer.digest.clone();

            view()
                .flex_col()
                .gap_2()
                .px_4()
                .py_2p5()
                .child(
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
                                .gap_2()
                                .child(ui_badge(format!("#{}", idx + 1), BadgeTone::Neutral))
                                .child(ui_opcode_pill(opcode))
                                .child(
                                    text(format!(
                                        "{} → {}",
                                        format_bytes(layer.size),
                                        format_bytes(layer.unpack_size)
                                    ))
                                    .text_xs()
                                    .font_family(font_mono_family())
                                    .tabular_nums()
                                    .text_color(move || theme().muted_foreground),
                                )
                                .child(ui_badge(
                                    format!(
                                        "+{} / ~{} / −{}",
                                        counts.added, counts.modified, counts.removed
                                    ),
                                    BadgeTone::Neutral,
                                )),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .when(hidden_count > 0, |r| {
                                    r.child(ui_button(
                                        move || {
                                            if command_expanded.get() {
                                                SharedString::from(t(lang.get(), "cmd.collapse"))
                                            } else {
                                                SharedString::from(tf(
                                                    lang.get(),
                                                    "cmd.show_more",
                                                    &[&hidden_count.to_string()],
                                                ))
                                            }
                                        },
                                        ButtonVariant::Ghost,
                                        UiSize::Xs,
                                        None,
                                        false,
                                        false,
                                        move || command_expanded.update(|v| *v = !*v),
                                    ))
                                })
                                .child(ui_button(
                                    move || SharedString::from(t(lang.get(), "cmd.wrap")),
                                    if command_wrap.get() {
                                        ButtonVariant::Default
                                    } else {
                                        ButtonVariant::Ghost
                                    },
                                    UiSize::Xs,
                                    None,
                                    false,
                                    false,
                                    move || command_wrap.update(|v| *v = !*v),
                                ))
                                .child(ui_button(
                                    move || {
                                        SharedString::from(if copied.get() {
                                            t(lang.get(), "common.copied")
                                        } else {
                                            t(lang.get(), "cmd.copy_digest")
                                        })
                                    },
                                    ButtonVariant::Outline,
                                    UiSize::Xs,
                                    Some(if copied.get() {
                                        IconName::Check
                                    } else {
                                        IconName::Copy
                                    }),
                                    false,
                                    false,
                                    move || {
                                        set_clipboard(digest_to_copy.clone());
                                        copied.set(true);
                                        set_timeout(Duration::from_millis(1500), move || {
                                            copied.set(false);
                                        });
                                    },
                                )),
                        ),
                )
                .child(
                    view()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(radius_inset)
                        .bg(move || theme().muted)
                        .border_1()
                        .border_color(move || theme().border)
                        .children(visible_lines.into_iter().map(move |line| {
                            let mut txt = text(line)
                                .text_xs()
                                .font_family(font_mono_family())
                                .text_color(move || theme().foreground);
                            if !command_wrap.get() {
                                txt = txt.number_of_lines(1);
                            }
                            txt
                        })),
                )
                .into_element()
        }))
}

fn filter_toolbar(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let expand_state = state.clone();
    let collapse_state = state.clone();
    let min_size_str = move || Some(SharedString::from(state.tree_min_size.get().to_string()));

    view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2p5()
        .px_4()
        .py_2()
        .bg(move || theme().card)
        .border_b_1()
        .border_color(move || theme().border)
        .child(view().flex_1().min_w(px(190.0)).child(ui_search_input(
            state.tree_keyword,
            move || t(lang.get(), "tree.filter_placeholder").to_owned(),
            "Filter files in layer",
            UiSize::Sm,
            Some("/"),
            Some(&state.tree_filter_ref),
            true,
            None,
        )))
        .child(ui_checkbox(
            move || SharedString::from(t(lang.get(), "tree.modifications_only")),
            state.tree_modifications_only,
            false,
            false,
            move |next| state.tree_modifications_only.set(next),
        ))
        .child(view().w(px(110.0)).child(ui_select(
            move || min_size_items(lang.get()),
            min_size_str,
            move || SharedString::from(t(lang.get(), "tree.size_all")),
            "Minimum file size filter",
            UiSize::Sm,
            false,
            move |next| {
                let parsed = next
                    .and_then(|s| s.as_str().parse::<u64>().ok())
                    .unwrap_or(0);
                state.tree_min_size.set(parsed);
            },
        )))
        .child(ui_checkbox(
            move || SharedString::from(t(lang.get(), "tree.compact_paths")),
            state.tree_compact_paths,
            false,
            false,
            move |next| state.tree_compact_paths.set(next),
        ))
        .child(ui_checkbox(
            move || SharedString::from(t(lang.get(), "tree.show_meta_cols")),
            state.tree_show_meta_cols,
            false,
            false,
            move |next| state.tree_show_meta_cols.set(next),
        ))
        .child(ui_button(
            move || SharedString::from(t(lang.get(), "tree.expand_all")),
            ButtonVariant::Ghost,
            UiSize::Xs,
            None,
            false,
            false,
            move || expand_state.expand_all_dirs(),
        ))
        .child(ui_button(
            move || SharedString::from(t(lang.get(), "tree.collapse_all")),
            ButtonVariant::Ghost,
            UiSize::Xs,
            None,
            false,
            false,
            move || collapse_state.collapse_all_dirs(),
        ))
}

fn column_header(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let show_meta = state.tree_show_meta_cols;

    view()
        .w_full()
        .h(px(28.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_4()
        .bg(move || theme().muted)
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            text(move || t(lang.get(), "tree.col_status"))
                .w(px(34.0))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_center()
                .text_color(move || theme().muted_foreground),
        )
        .child(when(show_meta, move || {
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t(lang.get(), "tree.col_mode"))
                        .w(px(76.0))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || t(lang.get(), "tree.col_uid"))
                        .w(px(58.0))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
        }))
        .child(
            text(move || t(lang.get(), "tree.col_size"))
                .w(px(70.0))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_right()
                .text_color(move || theme().muted_foreground),
        )
        .child(view().w(px(50.0)))
        .child(
            text(move || t(lang.get(), "tree.col_path"))
                .flex_1()
                .min_w(px(0.0))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().muted_foreground),
        )
}

fn virtual_tree(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    rows: Memo<Vec<FileTreeRow>>,
) -> impl IntoElement {
    let lang = state.lang;
    let selected_layer = state.selected_layer;
    let selected_file = state.selected_file_path;
    let show_meta = state.tree_show_meta_cols;
    let empty_res = res.clone();

    virtual_list(
        move || rows.with(Vec::len),
        move |at| {
            let press_state = state.clone();
            let preview_btn_state = state.clone();
            let ctx_preview_state = state.clone();
            let ctx_save_state = state.clone();
            let layer_res = res.clone();

            let row_view = view()
                .w_full()
                .min_h(px(28.0))
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .bg(move || {
                    let current_key = rows.with(|r| r.get(at.index()).map(|item| item.key.clone()));
                    if current_key.is_some() && current_key == selected_file.get() {
                        surface_active()
                    } else {
                        theme().background
                    }
                })
                .hover(|s| s.bg(move || theme().accent))
                .on_press(move || {
                    if let Some(row) = rows.with(|r| r.get(at.index()).cloned()) {
                        if row.is_dir {
                            press_state.toggle_tree_dir(&row.key, row.expanded);
                        } else if row.is_previewable() {
                            press_state.open_file_preview(&row);
                        } else {
                            press_state.selected_file_path.set(Some(row.key));
                        }
                    }
                })
                .child(
                    view()
                        .w(px(34.0))
                        .flex_shrink_0()
                        .items_center()
                        .child(dynamic(move || {
                            let op_kind =
                                rows.with(|r| r.get(at.index()).and_then(|row| row.op_kind()));
                            match op_kind {
                                Some(kind) => ui_status_badge(kind).into_element(),
                                None => {
                                    text("—").text_xs().text_color(text_tertiary).into_element()
                                }
                            }
                        })),
                )
                .child(when(show_meta, move || {
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .flex_shrink_0()
                        .child(
                            text(move || {
                                rows.with(|r| {
                                    r.get(at.index())
                                        .map(|row| row.mode.clone())
                                        .unwrap_or_default()
                                })
                            })
                            .w(px(76.0))
                            .text_xs()
                            .font_family(font_mono_family())
                            .number_of_lines(1)
                            .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text(move || {
                                rows.with(|r| {
                                    r.get(at.index())
                                        .map(|row| format!("{}:{}", row.uid, row.gid))
                                        .unwrap_or_default()
                                })
                            })
                            .w(px(58.0))
                            .text_xs()
                            .font_family(font_mono_family())
                            .number_of_lines(1)
                            .text_color(move || theme().muted_foreground),
                        )
                }))
                .child(
                    text(move || {
                        rows.with(|r| {
                            r.get(at.index())
                                .map(|row| format_bytes(row.size))
                                .unwrap_or_default()
                        })
                    })
                    .w(px(70.0))
                    .flex_shrink_0()
                    .text_xs()
                    .font_family(font_mono_family())
                    .tabular_nums()
                    .text_right()
                    .text_color(move || theme().foreground),
                )
                .child(view().w(px(50.0)).flex_shrink_0().child(dynamic(move || {
                    let ratio = rows.with(|r| r.get(at.index()).map_or(0.0, |row| row.ratio));
                    ui_size_bar(ratio, px(50.0))
                })))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .pl(move || {
                            let depth = rows.with(|r| r.get(at.index()).map_or(0, |row| row.depth));
                            px((depth as f32) * 14.0)
                        })
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(dynamic(move || {
                            let meta = rows.with(|r| {
                                r.get(at.index())
                                    .map(|row| (row.is_dir, row.expanded, !row.link.is_empty()))
                            });
                            match meta {
                                Some((true, expanded, _)) => view()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .child(ui_icon(
                                        if expanded {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        },
                                        px(12.0),
                                        move || theme().muted_foreground,
                                    ))
                                    .child(ui_icon(
                                        if expanded {
                                            IconName::FolderOpen
                                        } else {
                                            IconName::Folder
                                        },
                                        px(14.0),
                                        move || theme().primary,
                                    ))
                                    .into_element(),
                                Some((false, _, true)) => {
                                    ui_icon(IconName::Link, px(14.0), move || {
                                        theme().muted_foreground
                                    })
                                    .into_element()
                                }
                                _ => ui_icon(IconName::File, px(14.0), move || {
                                    theme().muted_foreground
                                })
                                .into_element(),
                            }
                        }))
                        .child(
                            text(move || {
                                rows.with(|r| {
                                    r.get(at.index())
                                        .map(|row| {
                                            if row.link.is_empty() {
                                                row.name.clone()
                                            } else {
                                                format!("{} -> {}", row.name, row.link)
                                            }
                                        })
                                        .unwrap_or_default()
                                })
                            })
                            .flex_1()
                            .min_w(px(0.0))
                            .text_xs()
                            .font_family(font_mono_family())
                            .number_of_lines(1)
                            .text_color(move || theme().foreground),
                        )
                        .child(when(
                            move || {
                                rows.with(|r| {
                                    r.get(at.index()).is_some_and(FileTreeRow::is_previewable)
                                })
                            },
                            move || {
                                let s = preview_btn_state.clone();
                                ui_icon_button(
                                    IconName::Eye,
                                    "Preview file contents",
                                    Some(move || SharedString::from(t(lang.get(), "tree.preview"))),
                                    ButtonVariant::Ghost,
                                    UiSize::Xs,
                                    false,
                                    false,
                                    move || {
                                        if let Some(row) = rows.with(|r| r.get(at.index()).cloned())
                                        {
                                            s.open_file_preview(&row);
                                        }
                                    },
                                )
                            },
                        )),
                );

            ui_context_menu(row_view, |_| {})
                .child(ui_menu_item(
                    move || SharedString::from(t(lang.get(), "tree.ctx_preview")),
                    Some(IconName::Eye),
                    None,
                    move || {
                        rows.with(|r| r.get(at.index()).is_none_or(|row| !row.is_previewable()))
                    },
                    move || {
                        if let Some(row) = rows.with(|r| r.get(at.index()).cloned()) {
                            ctx_preview_state.open_file_preview(&row);
                        }
                    },
                ))
                .child(ui_menu_item(
                    move || SharedString::from(t(lang.get(), "tree.ctx_copy_path")),
                    Some(IconName::Copy),
                    None,
                    false,
                    move || {
                        if let Some(row) = rows.with(|r| r.get(at.index()).cloned()) {
                            set_clipboard(row.key);
                        }
                    },
                ))
                .child(ui_menu_item(
                    move || SharedString::from(t(lang.get(), "tree.ctx_save_as")),
                    Some(IconName::Download),
                    None,
                    move || {
                        rows.with(|r| r.get(at.index()).is_none_or(|row| !row.is_previewable()))
                    },
                    move || {
                        if let Some(row) = rows.with(|r| r.get(at.index()).cloned())
                            && row.is_previewable()
                            && let Some(layer) =
                                layer_res.layers.get(selected_layer.get_untracked())
                        {
                            ctx_save_state.save_layer_file_at(
                                layer.digest.clone(),
                                layer.media_type.clone(),
                                row.key,
                            );
                        }
                    },
                ))
        },
    )
    .flex_1()
    .min_h(px(0.0))
    .estimated_item_size(px(28.0))
    .keys(move || {
        rows.with(|list| {
            list.iter()
                .map(|row| row.key.clone())
                .collect::<Vec<String>>()
        })
    })
    .empty(dynamic(move || {
        let layer_empty = empty_res
            .file_tree_list
            .get(selected_layer.get())
            .is_none_or(Vec::is_empty);
        let title_key = if layer_empty {
            "tree.empty_layer"
        } else {
            "tree.empty_filter"
        };
        ui_empty_state(
            IconName::Folder,
            move || SharedString::from(t(lang.get(), title_key)),
            "",
            None,
        )
    }))
}
