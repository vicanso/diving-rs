//! Tab 1 (`File Tree`): collapsible `CommandInspector`, filter toolbar, and `virtual_list` tree.

use std::sync::Arc;
use std::time::Duration;

use diving_core::DockerAnalyzeResult;
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::domain::{
    AppRoute, AppState, FileTreeFilter, FileTreeRow, IndexedTree, InspectorTab, SharedRef,
    build_indexed_tree, classify_layer_opcode, count_layer_file_ops, flatten_indexed_tree,
    format_bytes, format_command_lines,
};
use crate::i18n::{UiLang, t, tf};
use crate::ui::{
    BadgeTone, ButtonVariant, IconName, SelectItem, UiSize, font_mono_family, radius_inset,
    surface_active, text_tertiary, theme, ui_badge, ui_button, ui_checkbox, ui_context_menu,
    ui_empty_state, ui_icon, ui_icon_button, ui_menu_item, ui_opcode_pill,
    ui_search_input_with_escape, ui_select, ui_size_bar, ui_status_badge,
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
    let res_for_index = res.clone();

    // Build precomputed lowercase/flag index once per layer instead of per keystroke.
    let indexed_layer: Memo<Option<SharedRef<IndexedTree>>> = computed(move || {
        let layer_idx = selected_layer.get();
        res_for_index
            .file_tree_list
            .get(layer_idx)
            .map(|items| SharedRef::new(build_indexed_tree(items)))
    });

    // Debounce keyword typing by 120ms so fast typing stays responsive on large trees.
    let raw_keyword = state.tree_keyword;
    let debounced_keyword = signal(raw_keyword.get_untracked());
    let debounce_seq = signal(0_u64);
    effect(move || {
        let kw = raw_keyword.get();
        let next_seq = debounce_seq.get_untracked().wrapping_add(1);
        debounce_seq.set(next_seq);
        if kw.trim().is_empty() {
            debounced_keyword.set(kw);
        } else {
            set_timeout(Duration::from_millis(120), move || {
                if debounce_seq.try_get() == Some(next_seq) {
                    let _ = debounced_keyword.try_set(kw);
                }
            });
        }
    });

    let filter_state = state.clone();
    let rows: Memo<Vec<FileTreeRow>> = computed(move || {
        let Some(indexed) = indexed_layer.get() else {
            return Vec::new();
        };
        let filter = FileTreeFilter {
            keyword: debounced_keyword.get(),
            only_modified_removed: filter_state.tree_modifications_only.get(),
            min_size_bytes: filter_state.tree_min_size.get(),
            compact_paths: filter_state.tree_compact_paths.get(),
            expand_all: filter_state.tree_expand_all.get(),
        };
        let expanded = filter_state.tree_expanded.get();
        let collapsed = filter_state.tree_collapsed.get();
        flatten_indexed_tree(&indexed, &filter, &expanded, &collapsed)
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
    let copy_seq = signal(0_u64);

    effect(move || {
        let _ = selected_layer.get();
        copy_seq.set(copy_seq.get_untracked().wrapping_add(1));
        copied.set(false);
    });

    view()
        .w_full()
        .flex_col()
        .bg(move || theme().background)
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
            let digest_empty = digest_to_copy.is_empty();

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
                                .child(dynamic(move || {
                                    let variant = if command_wrap.get() {
                                        ButtonVariant::Default
                                    } else {
                                        ButtonVariant::Ghost
                                    };
                                    ui_button(
                                        move || SharedString::from(t(lang.get(), "cmd.wrap")),
                                        variant,
                                        UiSize::Xs,
                                        None,
                                        false,
                                        false,
                                        move || command_wrap.update(|v| *v = !*v),
                                    )
                                    .into_element()
                                }))
                                .child(dynamic(move || {
                                    let is_copied = copied.get();
                                    let digest = digest_to_copy.clone();
                                    ui_button(
                                        move || {
                                            SharedString::from(if copied.get() {
                                                t(lang.get(), "common.copied")
                                            } else {
                                                t(lang.get(), "cmd.copy_digest")
                                            })
                                        },
                                        ButtonVariant::Outline,
                                        UiSize::Xs,
                                        Some(if is_copied {
                                            IconName::Check
                                        } else {
                                            IconName::Copy
                                        }),
                                        digest_empty,
                                        false,
                                        move || {
                                            if digest.is_empty() {
                                                return;
                                            }
                                            set_clipboard(digest.clone());
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
                                })),
                        ),
                )
                .child(
                    view()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(radius_inset)
                        .bg(move || theme().muted)
                        .children(visible_lines.into_iter().map(move |line| {
                            text(line)
                                .text_xs()
                                .font_family(font_mono_family())
                                .text_color(move || theme().foreground)
                                .number_of_lines(
                                    move || if command_wrap.get() { 0_u32 } else { 1_u32 },
                                )
                        })),
                )
                .into_element()
        }))
}

fn filter_toolbar(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let expand_state = state.clone();
    let collapse_state = state.clone();
    let prov_state = state.clone();
    let esc_keyword = state.tree_keyword;
    let esc_ref = state.tree_filter_ref;
    let min_size_str = move || Some(SharedString::from(state.tree_min_size.get().to_string()));

    view()
        .w_full()
        .flex_col()
        .gap_1p5()
        .px_4()
        .py_2()
        .bg(move || theme().background)
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(220.0))
                        .child(ui_search_input_with_escape(
                            state.tree_keyword,
                            move || t(lang.get(), "tree.filter_placeholder").to_owned(),
                            "Filter files in layer",
                            UiSize::Sm,
                            Some("/"),
                            Some(&state.tree_filter_ref),
                            true,
                            None,
                            Some(Box::new(move || {
                                esc_keyword.set(String::new());
                                esc_ref.blur();
                            })),
                        )),
                )
                .child(view().w(px(108.0)).flex_shrink_0().child(ui_select(
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
                .child(ui_button(
                    move || SharedString::from(t(lang.get(), "provenance.button")),
                    ButtonVariant::Outline,
                    UiSize::Sm,
                    Some(IconName::Search),
                    false,
                    false,
                    move || prov_state.open_provenance_search(),
                )),
        )
        .child(
            view()
                .w_full()
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
                        .gap_3()
                        .child(ui_checkbox(
                            move || SharedString::from(t(lang.get(), "tree.modifications_only")),
                            state.tree_modifications_only,
                            false,
                            false,
                            move |next| state.tree_modifications_only.set(next),
                        ))
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
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .flex_shrink_0()
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "tree.expand_all")),
                            ButtonVariant::Ghost,
                            UiSize::Sm,
                            None,
                            false,
                            false,
                            move || expand_state.expand_all_dirs(),
                        ))
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "tree.collapse_all")),
                            ButtonVariant::Ghost,
                            UiSize::Sm,
                            None,
                            false,
                            false,
                            move || collapse_state.collapse_all_dirs(),
                        )),
                ),
        )
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
        .bg(move || theme().background)
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

fn step_tree_selection(
    rows: &Memo<Vec<FileTreeRow>>,
    state: &AppState,
    list_ref: &ElementRef,
    delta: isize,
) {
    rows.with_untracked(|list| {
        if list.is_empty() {
            return;
        }
        let cur_pos = state
            .selected_file_path
            .get_untracked()
            .and_then(|sel| list.iter().position(|row| row.key == sel));
        let next_pos = match cur_pos {
            Some(pos) => (pos as isize + delta).clamp(0, (list.len() - 1) as isize) as usize,
            None if delta >= 0 => 0,
            None => list.len() - 1,
        };
        if let Some(row) = list.get(next_pos) {
            state.selected_file_path.set(Some(row.key.clone()));
            list_ref.scroll_to_index(next_pos as u32, Percentage(0.5), px(0.0), false);
        }
    });
}

fn toggle_selected_tree_dir(rows: &Memo<Vec<FileTreeRow>>, state: &AppState, expand: bool) {
    let Some(sel) = state.selected_file_path.get_untracked() else {
        return;
    };
    let row = rows.with_untracked(|list| list.iter().find(|r| r.key == sel).cloned());
    if let Some(row) = row
        && row.is_dir
        && row.expanded != expand
    {
        state.toggle_tree_dir(&row.key, row.expanded);
    }
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

    let list_ref = element_ref();
    let focus_in_state = state.clone();
    list_ref.on_focus_in(move || focus_in_state.file_tree_focused.set(true));
    let focus_out_state = state.clone();
    list_ref.on_focus_out(move || focus_out_state.file_tree_focused.set(false));
    let cleanup_state = state.clone();
    cleanup(move || cleanup_state.file_tree_focused.set(false));

    let active_state = state.clone();
    let tree_active = computed(move || {
        active_state.file_tree_focused.get()
            && !active_state.command_palette_open.get()
            && !active_state.provenance_open.get()
            && !active_state.compare_dialog_open.get()
            && !active_state.local_picker_open.get()
            && !active_state.preview_sheet_open.get()
            && active_state.route.get() == AppRoute::Inspector
            && active_state.active_tab.get() == InspectorTab::FileTree
    });
    let nav_state = state.clone();
    effect(move || {
        if !tree_active.get() {
            return;
        }
        let tree_nav = KeyOptions {
            when: When::NotEditing,
            repeat: true,
            for_focus: true,
            ..Default::default()
        };
        let up_state = nav_state.clone();
        let up_list = list_ref;
        on_key_with("up", tree_nav, move |_| {
            step_tree_selection(&rows, &up_state, &up_list, -1);
        });
        let down_state = nav_state.clone();
        let down_list = list_ref;
        on_key_with("down", tree_nav, move |_| {
            step_tree_selection(&rows, &down_state, &down_list, 1);
        });
        let right_state = nav_state.clone();
        on_key_with("right", tree_nav, move |_| {
            toggle_selected_tree_dir(&rows, &right_state, true);
        });
        let left_state = nav_state.clone();
        on_key_with("left", tree_nav, move |_| {
            toggle_selected_tree_dir(&rows, &left_state, false);
        });
    });

    virtual_list(
        move || rows.with(Vec::len),
        move |at| {
            let focus_state = state.clone();
            let press_state = state.clone();
            let key_state = state.clone();
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
                .focusable(true)
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .bg(move || {
                    let current_key = rows.with(|r| r.get(at.index()).map(|item| item.key.clone()));
                    if current_key.is_some() && current_key == selected_file.get() {
                        surface_active()
                    } else {
                        theme().background
                    }
                })
                .hover(|s| s.bg(move || theme().accent))
                .on_focus(move || {
                    if let Some(key) =
                        rows.with_untracked(|r| r.get(at.index()).map(|n| n.key.clone()))
                    {
                        focus_state.selected_file_path.set(Some(key));
                    }
                })
                .on_key_down(move |ev: &KeyEvent| {
                    let idx = at.index();
                    let Some(row) = rows.with(|r| r.get(idx).cloned()) else {
                        return;
                    };
                    if matches!(ev.key.as_str(), "Enter" | " ") {
                        if row.is_dir {
                            key_state.toggle_tree_dir(&row.key, row.expanded);
                        } else if row.is_previewable() {
                            key_state.open_file_preview(&row);
                        } else {
                            key_state.selected_file_path.set(Some(row.key));
                        }
                    }
                })
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
                            px((depth as f32) * 16.0)
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
    .element_ref(&list_ref)
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use diving_core::{Analyzer, FileTreeItem, ImageLayer, Op};
    use nsh_core::clipboard::{ClipboardBackend, MemoryClipboard};
    use nsh_engine::{Clock, EngineConfig, Environment, Idiom, Os, Points, Root, Size};
    use nsh_testing::Headless;

    use super::*;
    use crate::domain::DivingService;
    use crate::features::install_desktop_menus_and_keys;
    use crate::ui::app_tokens;

    fn file_tree_fixture() -> Arc<DockerAnalyzeResult> {
        Arc::new(DockerAnalyzeResult {
            layers: vec![
                ImageLayer {
                    digest: "sha256:layer0".to_owned(),
                    cmd: "COPY etc /etc".to_owned(),
                    size: 120,
                    unpack_size: 240,
                    empty: false,
                    ..Default::default()
                },
                ImageLayer {
                    digest: String::new(),
                    cmd: "ENV FOO=bar".to_owned(),
                    size: 0,
                    unpack_size: 0,
                    empty: true,
                    ..Default::default()
                },
            ],
            file_tree_list: vec![
                vec![FileTreeItem {
                    name: "etc".to_owned(),
                    size: 120,
                    op: Op::Added,
                    mode: "drwxr-xr-x".to_owned(),
                    children: vec![FileTreeItem {
                        name: "app.conf".to_owned(),
                        size: 120,
                        op: Op::Added,
                        mode: "-rw-r--r--".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                Vec::new(),
            ],
            ..Default::default()
        })
    }

    #[component]
    fn FileTreeHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let res = file_tree_fixture();
        state.result.set(Some(res.clone()));
        state.selected_layer.set(0);
        state.tree_expand_all.set(true);
        let mounted = signal(true);
        let switch_state = state.clone();

        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(ui_button(
                        "Switch layer 1",
                        ButtonVariant::Ghost,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        move || switch_state.select_layer(1),
                    ))
                    .child(ui_button(
                        "Unmount file tree",
                        ButtonVariant::Ghost,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        move || mounted.set(false),
                    )),
            )
            .child(when(mounted, move || {
                file_tree_tab(state.clone(), res.clone())
            }))
    }

    #[test]
    fn copy_digest_timer_safe_on_unmount_resets_on_layer_switch_and_disables_when_empty() {
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
            ..EngineConfig::new(Root::of::<FileTreeHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start file_tree harness");

        // Verify filter placeholder is not truncated and Copy digest works & resets on layer switch.
        app.wait_for_within("Copy digest", Duration::from_secs(2))
            .expect("Copy digest visible");
        assert_eq!(app.screen().enabled("Copy digest"), Ok(true));

        // Verify child row (`app.conf`) aligns its icon at +16px from parent folder (`etc`) chevron.
        app.wait_for_within("app.conf", Duration::from_secs(2))
            .expect("child row visible");
        let nodes = app.screen().layout_nodes();
        let etc_x = nodes
            .iter()
            .find(|n| n.text.as_ref().is_some_and(|t| t.text.text == "etc"))
            .map(|n| n.bounds.origin.x.0)
            .expect("etc text node");
        let app_x = nodes
            .iter()
            .find(|n| n.text.as_ref().is_some_and(|t| t.text.text == "app.conf"))
            .map(|n| n.bounds.origin.x.0)
            .expect("app.conf text node");
        // Parent folder has chevron(12)+gap(4)+folder(14)+gap(6)=36 before text;
        // child file at depth 1 has pl(16)+file(14)+gap(6)=36 before text -> exact 0px offset!
        assert!(
            (etc_x - app_x).abs() < 0.5,
            "child file text ({app_x}) must align on same spine as parent folder text ({etc_x})"
        );

        app.press("Copy digest").expect("press Copy digest");
        assert_eq!(clipboard.text().as_deref(), Some("sha256:layer0"));
        app.wait_for_within("Copied", Duration::from_secs(1))
            .expect("Copied state shown");

        // Switching to layer 1 resets Copied immediately and disables Copy digest for empty digest.
        app.press("Switch layer 1").expect("switch to layer 1");
        app.wait_for_within("Copy digest", Duration::from_secs(2))
            .expect("Copied reset on layer switch");
        assert_eq!(app.screen().enabled("Copy digest"), Ok(false));

        // Unmounting within 1.5s after copying on layer 0 must not panic.
        app.press("Unmount file tree").expect("unmount tab");
        app.wait_for_gone("Copy digest", Duration::from_secs(2))
            .expect("file tree unmounted");
        app.fast_forward(Duration::from_millis(1600))
            .expect("1.5s copy timer after file tree unmount must not panic");
    }

    #[test]
    fn filter_debounce_timer_safe_on_tab_switch_within_120ms() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            clock: Clock::manual(SystemTime::UNIX_EPOCH),
            ..EngineConfig::new(Root::of::<FileTreeHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start file_tree debounce harness");
        app.wait_for_within("Copy digest", Duration::from_secs(2))
            .expect("file tree toolbar ready");

        app.type_text("Filter files in layer", "app.conf")
            .expect("type filter keyword");
        app.press("Unmount file tree")
            .expect("unmount file tree within 120ms");
        app.wait_for_gone("Copy digest", Duration::from_secs(2))
            .expect("file tree unmounted");
        app.fast_forward(Duration::from_millis(200))
            .expect("120ms filter debounce timer after unmount must not panic");
    }

    #[component]
    fn FileTreeFocusNavHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        install_desktop_menus_and_keys(state.clone());
        let res = file_tree_fixture();
        state.result.set(Some(res.clone()));
        state.route.set(AppRoute::Inspector);
        state.active_tab.set(InspectorTab::FileTree);
        state.selected_layer.set(0);
        state.tree_expand_all.set(true);

        let lang = state.lang;
        let selected_layer = state.selected_layer;
        let selected_file = state.selected_file_path;
        let toggle_lang_state = state.clone();

        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(text(move || {
                        format!(
                            "layer={};file={}",
                            selected_layer.get(),
                            selected_file.get().unwrap_or_default()
                        )
                    }))
                    .child(ui_button(
                        move || {
                            SharedString::from(match lang.get() {
                                UiLang::En => "Outside focus",
                                UiLang::Zh => "外部焦点",
                            })
                        },
                        ButtonVariant::Outline,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        move || toggle_lang_state.set_lang(UiLang::Zh),
                    )),
            )
            .child(file_tree_tab(state, res))
    }

    #[test]
    fn focused_file_tree_arrows_navigate_tree_without_stepping_layers() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(Root::of::<FileTreeFocusNavHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start file_tree focus nav harness");
        app.wait_for_within("app.conf", Duration::from_secs(2))
            .expect("file tree rows rendered");
        assert!(app.shows("layer=0;file="));

        // Before FileTree is focused, Down/Up step layers globally.
        app.press_key("down").expect("step down to layer 1");
        app.wait_for_within("layer=1;file=", Duration::from_secs(2))
            .expect("layer stepped to 1");
        app.press_key("up").expect("step back up to layer 0");
        app.wait_for_within("layer=0;file=", Duration::from_secs(2))
            .expect("layer stepped back to 0");
        app.wait_for_within("app.conf", Duration::from_secs(2))
            .expect("layer 0 rows visible again");

        // Tab until focus enters the virtualized FileTree (`etc` row gets selected on focus).
        for _ in 0..20 {
            app.press_key("tab").expect("tab forward");
            if app.shows("layer=0;file=etc") {
                break;
            }
        }
        assert!(
            app.shows("layer=0;file=etc"),
            "Tab should reach FileTree row `etc`"
        );

        // Inside focused FileTree, Down/Up move selected file row and NEVER step layer.
        app.press_key("down").expect("move down inside file tree");
        app.wait_for_within("layer=0;file=etc/app.conf", Duration::from_secs(2))
            .expect("arrow down selected child row without stepping layer");
        app.press_key("up").expect("move up inside file tree");
        app.wait_for_within("layer=0;file=etc", Duration::from_secs(2))
            .expect("arrow up selected parent row without stepping layer");

        // Moving focus back out of FileTree (Shift+Tab onto toolbar `Collapse all`) restores global layer stepping.
        app.press_key("shift+tab").expect("tab back out of tree");
        app.press_key("down")
            .expect("step layer down once outside file tree");
        app.wait_for_within("layer=1;file=", Duration::from_secs(2))
            .expect("leaving FileTree focus must restore global layer stepping");
    }
}
