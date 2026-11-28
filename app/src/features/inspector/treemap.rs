//! Tab 2 (`Treemap`): final-filesystem squarified spatial map, breadcrumbs, layer legend, and Table view.

use std::sync::Arc;

use diving_core::{DockerAnalyzeResult, FileTreeItem};
use nsh_core::AccessibilityState;
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::domain::{
    AppRoute, AppState, FileTreeRow, FsNode, InspectorTab, SharedRef, TreemapHoverInfo,
    TreemapTile, TreemapViewMode, dominant_layer, find_fs_node, format_bytes, layout_treemap_tiles,
    slot_for_layer, top_three_layers,
};
use crate::features::sidebar::APP_SIDEBAR_WIDTH;
use crate::i18n::{t, tf};
use crate::ui::{
    BadgeTone, ButtonVariant, IconName, SegmentOption, UiSize, font_mono_family, radius_control,
    radius_inset, surface_active, theme, treemap_ink, treemap_slot, ui_badge, ui_button,
    ui_empty_state, ui_icon, ui_segmented, ui_spinner, ui_treemap_size_bar,
};

const MIN_LABEL_W: f32 = 56.0;
const MIN_LABEL_H: f32 = 24.0;
const MIN_SIZE_LABEL_W: f32 = 74.0;
const MIN_SIZE_LABEL_H: f32 = 38.0;

/// Renders the `Treemap` workspace tab with Map/Table toggle, drill-down breadcrumbs, and Top-3 layer legend.
pub fn treemap_tab(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let lang = state.lang;
    let final_tree = state.final_tree;

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_col()
        .child(dynamic(move || {
            let Some(root_node) = final_tree.get() else {
                return view()
                    .flex_1()
                    .min_h(px(0.0))
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(ui_spinner(px(20.0), move || theme().primary))
                    .child(
                        text(move || t(lang.get(), "landing.analyzing"))
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
                    .into_element();
            };
            let top_layers = top_three_layers(&root_node.0);

            view()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex_col()
                .child(treemap_toolbar(
                    state.clone(),
                    root_node.clone(),
                    top_layers.clone(),
                ))
                .child(treemap_body(
                    state.clone(),
                    res.clone(),
                    root_node,
                    top_layers,
                ))
                .child(treemap_info_footer(state.clone()))
                .into_element()
        }))
}

fn treemap_toolbar(
    state: AppState,
    root: SharedRef<FsNode>,
    top_layers: Vec<(usize, u64)>,
) -> impl IntoElement {
    let lang = state.lang;
    let mode_state = state.clone();
    let crumb_state = state.clone();
    let total_size = root.0.size.max(1);

    view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        .px_4()
        .py_2()
        .bg(move || theme().background)
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap_1()
                .child(dynamic(move || {
                    let s = crumb_state.clone();
                    let path = s.treemap_path.get();
                    let root_s = s.clone();
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "treemap.root")),
                            ButtonVariant::Ghost,
                            UiSize::Xs,
                            Some(IconName::Treemap),
                            false,
                            false,
                            move || {
                                root_s.treemap_path.set(Vec::new());
                                root_s.treemap_hovered.set(None);
                            },
                        ))
                        .children(path.iter().enumerate().map(|(idx, seg)| {
                            let step_s = s.clone();
                            let seg_label = seg.clone();
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .child(
                                    text("/")
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(ui_button(
                                    SharedString::from(seg_label),
                                    ButtonVariant::Ghost,
                                    UiSize::Xs,
                                    None,
                                    false,
                                    false,
                                    move || {
                                        let mut next = step_s.treemap_path.get_untracked();
                                        next.truncate(idx + 1);
                                        step_s.treemap_path.set(next);
                                        step_s.treemap_hovered.set(None);
                                    },
                                ))
                        }))
                })),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2p5()
                .children(
                    top_layers
                        .iter()
                        .enumerate()
                        .map(|(pos, &(layer_idx, bytes))| {
                            let slot = pos + 1;
                            let pct = ((bytes as f64 / total_size as f64) * 100.0).round() as u64;
                            layer_legend_pill(slot, format!("L{} · {pct}%", layer_idx + 1))
                        }),
                )
                .when(top_layers.len() == 3, |row| {
                    row.child(dynamic(move || {
                        layer_legend_pill(0, t(lang.get(), "treemap.other_layers").to_owned())
                    }))
                })
                .child(ui_segmented(
                    vec![
                        SegmentOption::new(TreemapViewMode::Map, move || {
                            SharedString::from(t(lang.get(), "treemap.mode_map"))
                        }),
                        SegmentOption::new(TreemapViewMode::Table, move || {
                            SharedString::from(t(lang.get(), "treemap.mode_table"))
                        }),
                    ],
                    state.treemap_mode,
                    UiSize::Sm,
                    "Treemap display mode",
                    move |next| mode_state.treemap_mode.set(next),
                )),
        )
}

fn layer_legend_pill(slot: usize, label: String) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1p5()
        .px_2()
        .py_0p5()
        .rounded(radius_inset)
        .bg(move || theme().muted)
        .border_1()
        .border_color(move || theme().border)
        .child(
            view()
                .size(px(10.0))
                .rounded(radius_inset)
                .bg(move || treemap_slot(slot))
                .accessibility_elements_hidden(true),
        )
        .child(
            text(label)
                .text_xs()
                .font_family(font_mono_family())
                .tabular_nums()
                .text_color(move || theme().foreground),
        )
}

fn treemap_body(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    root: SharedRef<FsNode>,
    top_layers: Vec<(usize, u64)>,
) -> impl IntoElement {
    let top_layer_ids: Vec<usize> = top_layers.into_iter().map(|(id, _)| id).collect();
    let size_state = state.clone();
    let measured_canvas = signal(None::<(f32, f32)>);
    let canvas_size = computed(move || {
        if let Some((w, h)) = measured_canvas.get() {
            return (w, h);
        }
        let win = window();
        let app_sidebar_w = if size_state.app_sidebar_open.get() {
            APP_SIDEBAR_WIDTH
        } else {
            0.0
        };
        let raw_w = (win.width.0 - app_sidebar_w - size_state.sidebar_width.get().0 - 40.0)
            .clamp(320.0, 1280.0);
        let raw_h = (win.height.0 - 216.0).clamp(260.0, 680.0);
        (raw_w.floor().max(280.0), raw_h.floor().max(200.0))
    });

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .flex_col()
        .on_layout(move |bounds: &Bounds<Points>| {
            let next_w = (bounds.size.width.0 - 32.0).floor().max(280.0);
            let next_h = (bounds.size.height.0 - 32.0).floor().max(200.0);
            let changed = measured_canvas
                .get_untracked()
                .is_none_or(|(pw, ph)| (pw - next_w).abs() > 1.0 || (ph - next_h).abs() > 1.0);
            if changed {
                measured_canvas.set(Some((next_w, next_h)));
            }
        })
        .child(dynamic(move || {
            let lang = state.lang;
            let segments = state.treemap_path.get();
            let current = find_fs_node(&root.0, &segments);
            if current.children.is_empty() || current.size == 0 {
                return view()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(ui_empty_state(
                        IconName::Treemap,
                        move || SharedString::from(t(lang.get(), "treemap.empty")),
                        "",
                        None,
                    ))
                    .into_element();
            }

            match state.treemap_mode.get() {
                TreemapViewMode::Map => {
                    let (w, h) = canvas_size.get();
                    treemap_canvas_view(state.clone(), res.clone(), current, &top_layer_ids, w, h)
                        .into_element()
                }
                TreemapViewMode::Table => {
                    treemap_table_view(state.clone(), res.clone(), current, &top_layer_ids)
                        .into_element()
                }
            }
        }))
}

fn treemap_canvas_view(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    node: &FsNode,
    top_layer_ids: &[usize],
    w: f32,
    h: f32,
) -> impl IntoElement {
    let tiles = layout_treemap_tiles(node, w, h, top_layer_ids);
    let parent_size = node.size.max(1);
    let leave_state = state.clone();

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .p_4()
        .items_center()
        .justify_center()
        .child(
            view()
                .relative()
                .w(px(w))
                .h(px(h))
                .rounded(radius_control)
                .overflow_hidden()
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().muted)
                .on_mouse_leave(move |_: &MouseEvent| leave_state.treemap_hovered.set(None))
                .children(tiles.into_iter().map(move |tile| {
                    render_treemap_tile(state.clone(), res.clone(), tile, parent_size)
                })),
        )
}

fn render_treemap_tile(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    tile: TreemapTile,
    parent_size: u64,
) -> impl IntoElement {
    let rect = tile.rect;
    let slot = tile.color_slot;
    let share = (tile.size as f64 / parent_size as f64) as f32;
    let hover_info = TreemapHoverInfo {
        name: tile.name.clone(),
        path: tile.path.clone(),
        size: tile.size,
        layer: tile.layer,
        share,
    };
    let hover_state = state.clone();
    let click_state = state.clone();
    let key_state = state.clone();
    let key_res = res.clone();
    let key_path = tile.path.clone();
    let tile_path = tile.path.clone();
    let tile_layer = tile.layer;
    let is_dir = tile.is_dir;
    let has_label = rect.w >= MIN_LABEL_W && rect.h >= MIN_LABEL_H;
    let has_size = rect.w >= MIN_SIZE_LABEL_W && rect.h >= MIN_SIZE_LABEL_H;
    let a11y_label = format!("{} ({})", tile.path, format_bytes(tile.size));
    let tile_name = tile.name.clone();
    let tile_size_label = format_bytes(tile.size);

    view()
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.w))
        .h(px(rect.h))
        .overflow_hidden()
        .rounded(radius_inset)
        .bg(move || treemap_slot(slot))
        .border_1()
        .border_color(move || theme().background)
        .role(AccessibilityRole::Button)
        .label(a11y_label)
        .focusable(true)
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .hover(|s| s.opacity(0.92))
        .on_mouse_enter(move |_: &MouseEvent| {
            hover_state.treemap_hovered.set(Some(hover_info.clone()));
        })
        .on_key_down(move |ev: &KeyEvent| {
            if matches!(ev.key.as_str(), "Enter" | " ") {
                if is_dir {
                    key_state.treemap_path.set(split_segments(&key_path));
                    key_state.treemap_hovered.set(None);
                } else {
                    open_treemap_file(&key_state, &key_res, &key_path, tile_layer);
                }
            }
        })
        .on_press(move || {
            if is_dir {
                click_state.treemap_path.set(split_segments(&tile_path));
                click_state.treemap_hovered.set(None);
            } else {
                open_treemap_file(&click_state, &res, &tile_path, tile_layer);
            }
        })
        .when(has_label, |cell| {
            cell.child(
                view()
                    .p_1p5()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        text(tile_name)
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(1)
                            .text_color(move || treemap_ink(slot == 0)),
                    )
                    .when(has_size && tile.children.is_empty(), |col| {
                        col.child(
                            text(tile_size_label)
                                .text_xs()
                                .font_family(font_mono_family())
                                .tabular_nums()
                                .number_of_lines(1)
                                .text_color(move || treemap_ink(slot == 0).opacity(0.90)),
                        )
                    }),
            )
        })
        .children(tile.children.into_iter().map(|child| {
            let sub_rect = child.rect;
            let sub_slot = child.color_slot;
            let show_sub_label = sub_rect.w >= 54.0 && sub_rect.h >= 22.0;
            let show_sub_size = sub_rect.w >= 120.0 && sub_rect.h >= 24.0;
            let sub_size_label = format_bytes(child.size);
            view()
                .absolute()
                .left(px(sub_rect.x))
                .top(px(sub_rect.y))
                .w(px(sub_rect.w))
                .h(px(sub_rect.h))
                .overflow_hidden()
                .rounded(radius_inset)
                .bg(move || treemap_slot(sub_slot))
                .border_1()
                .border_color(move || theme().background)
                .when(show_sub_label, |sub| {
                    sub.p_1().child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_1p5()
                            .child(
                                text(child.name)
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .number_of_lines(1)
                                    .text_color(move || treemap_ink(sub_slot == 0)),
                            )
                            .when(show_sub_size, |row| {
                                row.child(
                                    text(sub_size_label)
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .tabular_nums()
                                        .number_of_lines(1)
                                        .text_color(move || {
                                            treemap_ink(sub_slot == 0).opacity(0.90)
                                        }),
                                )
                            }),
                    )
                })
        }))
}

#[derive(Clone, PartialEq)]
struct TreemapTableRow {
    name: String,
    path: String,
    size: u64,
    is_dir: bool,
    layer: Option<usize>,
    share: f32,
    color_slot: usize,
}

fn treemap_table_rows(node: &FsNode, top_layer_ids: &[usize]) -> Arc<Vec<TreemapTableRow>> {
    let total_size = node.size.max(1);
    let mut children: Vec<&FsNode> = node.children.values().filter(|c| c.size > 0).collect();
    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    let rows = children
        .into_iter()
        .map(|entry| {
            let dom = dominant_layer(entry);
            let layer = dom.map(|(l, _)| l).or(entry.layer);
            let share = (entry.size as f64 / total_size as f64) as f32;
            let color_slot = slot_for_layer(layer, top_layer_ids);
            TreemapTableRow {
                name: entry.name.clone(),
                path: entry.path.clone(),
                size: entry.size,
                is_dir: entry.is_dir(),
                layer,
                share,
                color_slot,
            }
        })
        .collect();
    Arc::new(rows)
}

fn treemap_table_view(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    node: &FsNode,
    top_layer_ids: &[usize],
) -> impl IntoElement {
    let items = treemap_table_rows(node, top_layer_ids);
    treemap_table_list(state, res, signal(items))
}

fn step_treemap_table_selection(
    rows: Signal<Arc<Vec<TreemapTableRow>>>,
    selected_path: Signal<Option<String>>,
    state: &AppState,
    list_ref: &ElementRef,
    delta: isize,
) {
    rows.with_untracked(|list| {
        if list.is_empty() {
            return;
        }
        let cur_pos = selected_path
            .get_untracked()
            .and_then(|sel| list.iter().position(|row| row.path == sel));
        let next_pos = match cur_pos {
            Some(pos) => (pos as isize + delta).clamp(0, (list.len() - 1) as isize) as usize,
            None if delta >= 0 => 0,
            None => list.len() - 1,
        };
        if let Some(entry) = list.get(next_pos) {
            selected_path.set(Some(entry.path.clone()));
            state.treemap_hovered.set(Some(TreemapHoverInfo {
                name: entry.name.clone(),
                path: entry.path.clone(),
                size: entry.size,
                layer: entry.layer,
                share: entry.share,
            }));
            list_ref.scroll_to_index(next_pos as u32, Percentage(0.5), px(0.0), false);
        }
    });
}

fn activate_treemap_table_row(
    rows: Signal<Arc<Vec<TreemapTableRow>>>,
    selected_path: Signal<Option<String>>,
    state: &AppState,
    res: &Arc<DockerAnalyzeResult>,
    fallback_idx: Option<usize>,
) {
    let entry = rows.with_untracked(|list| {
        if let Some(sel) = selected_path.get_untracked()
            && let Some(active) = list.iter().find(|r| r.path == sel)
        {
            return Some(active.clone());
        }
        fallback_idx.and_then(|idx| list.get(idx).cloned())
    });
    let Some(entry) = entry else {
        return;
    };
    if entry.is_dir {
        state.treemap_path.set(split_segments(&entry.path));
        state.treemap_hovered.set(None);
    } else {
        open_treemap_file(state, res, &entry.path, entry.layer);
    }
}

fn treemap_table_list(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    rows: Signal<Arc<Vec<TreemapTableRow>>>,
) -> impl IntoElement {
    let lang = state.lang;
    let selected_path = signal(None::<String>);
    let table_focused = signal(false);
    let list_ref = element_ref();

    list_ref.on_focus_in(move || table_focused.set(true));
    list_ref.on_focus_out(move || table_focused.set(false));
    cleanup(move || table_focused.set(false));

    let active_state = state.clone();
    let table_active = computed(move || {
        table_focused.get()
            && !active_state.any_overlay_open()
            && active_state.route.get() == AppRoute::Inspector
            && active_state.active_tab.get() == InspectorTab::Treemap
    });
    let nav_state = state.clone();
    let nav_res = res.clone();
    effect(move || {
        if !table_active.get() {
            return;
        }
        let nav_opts = KeyOptions {
            when: When::NotEditing,
            repeat: true,
            for_focus: true,
            ..Default::default()
        };
        let up_state = nav_state.clone();
        let up_list = list_ref;
        on_key_with("up", nav_opts, move |_| {
            step_treemap_table_selection(rows, selected_path, &up_state, &up_list, -1);
        });
        let down_state = nav_state.clone();
        let down_list = list_ref;
        on_key_with("down", nav_opts, move |_| {
            step_treemap_table_selection(rows, selected_path, &down_state, &down_list, 1);
        });
        let act_opts = KeyOptions {
            when: When::NotEditing,
            repeat: false,
            for_focus: true,
            ..Default::default()
        };
        let enter_state = nav_state.clone();
        let enter_res = nav_res.clone();
        on_key_with("enter", act_opts, move |_| {
            activate_treemap_table_row(rows, selected_path, &enter_state, &enter_res, None);
        });
        let space_state = nav_state.clone();
        let space_res = nav_res.clone();
        on_key_with("space", act_opts, move |_| {
            activate_treemap_table_row(rows, selected_path, &space_state, &space_res, None);
        });
    });

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .flex_col()
        .child(
            view()
                .w_full()
                .h(px(28.0))
                .flex_row()
                .items_center()
                .gap_3()
                .px_4()
                .bg(move || theme().muted)
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    text(move || t(lang.get(), "treemap.col_name"))
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || t(lang.get(), "treemap.col_layer"))
                        .w(px(72.0))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || t(lang.get(), "treemap.col_share"))
                        .w(px(120.0))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_right()
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || t(lang.get(), "treemap.col_size"))
                        .w(px(88.0))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_right()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            virtual_list(
                move || rows.with(|list| list.len()),
                move |at| {
                    let click_state = state.clone();
                    let click_res = res.clone();
                    let focus_state = state.clone();
                    let key_state = state.clone();
                    let key_res = res.clone();
                    let hover_state = state.clone();

                    view()
                        .w_full()
                        .min_h(px(32.0))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .border_b_1()
                        .border_color(move || theme().border.opacity(0.45))
                        .role(AccessibilityRole::Button)
                        .focusable(true)
                        .accessibility_state(move || {
                            let cur =
                                rows.with(|list| list.get(at.index()).map(|e| e.path.clone()));
                            let is_sel = cur.is_some() && cur == selected_path.get();
                            AccessibilityState {
                                selected: is_sel,
                                ..AccessibilityState::default()
                            }
                        })
                        .selected(|s| s.bg(surface_active))
                        .bg(move || {
                            let cur =
                                rows.with(|list| list.get(at.index()).map(|e| e.path.clone()));
                            if cur.is_some() && cur == selected_path.get() {
                                surface_active()
                            } else {
                                theme().background
                            }
                        })
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .hover(|s| s.bg(move || theme().accent))
                        .on_focus(move || {
                            if let Some(entry) =
                                rows.with_untracked(|list| list.get(at.index()).cloned())
                            {
                                selected_path.set(Some(entry.path.clone()));
                                focus_state.treemap_hovered.set(Some(TreemapHoverInfo {
                                    name: entry.name,
                                    path: entry.path,
                                    size: entry.size,
                                    layer: entry.layer,
                                    share: entry.share,
                                }));
                            }
                        })
                        .on_mouse_enter(move |_: &MouseEvent| {
                            if let Some(entry) = rows.with(|list| list.get(at.index()).cloned()) {
                                hover_state.treemap_hovered.set(Some(TreemapHoverInfo {
                                    name: entry.name,
                                    path: entry.path,
                                    size: entry.size,
                                    layer: entry.layer,
                                    share: entry.share,
                                }));
                            }
                        })
                        .on_key_down(move |ev: &KeyEvent| {
                            if matches!(ev.key.as_str(), "Enter" | "Space" | " ") {
                                activate_treemap_table_row(
                                    rows,
                                    selected_path,
                                    &key_state,
                                    &key_res,
                                    Some(at.index()),
                                );
                            }
                        })
                        .on_press(move || {
                            if let Some(entry) = rows.with(|list| list.get(at.index()).cloned()) {
                                selected_path.set(Some(entry.path.clone()));
                                if entry.is_dir {
                                    click_state.treemap_path.set(split_segments(&entry.path));
                                    click_state.treemap_hovered.set(None);
                                } else {
                                    open_treemap_file(
                                        &click_state,
                                        &click_res,
                                        &entry.path,
                                        entry.layer,
                                    );
                                }
                            }
                        })
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(dynamic(move || {
                                    let is_dir = rows.with(|list| {
                                        list.get(at.index()).is_some_and(|e| e.is_dir)
                                    });
                                    ui_icon(
                                        if is_dir {
                                            IconName::Folder
                                        } else {
                                            IconName::File
                                        },
                                        px(14.0),
                                        move || {
                                            if is_dir {
                                                theme().primary
                                            } else {
                                                theme().muted_foreground
                                            }
                                        },
                                    )
                                    .into_element()
                                }))
                                .child(
                                    text(move || {
                                        rows.with(|list| {
                                            list.get(at.index())
                                                .map(|e| e.name.clone())
                                                .unwrap_or_default()
                                        })
                                    })
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .font_family(font_mono_family())
                                    .number_of_lines(1)
                                    .text_color(move || theme().foreground),
                                ),
                        )
                        .child(view().w(px(72.0)).items_start().child(ui_badge(
                            move || {
                                let label = rows.with(|list| {
                                    list.get(at.index())
                                        .and_then(|e| e.layer)
                                        .map_or_else(|| "—".to_owned(), |l| format!("L{}", l + 1))
                                });
                                SharedString::from(label)
                            },
                            BadgeTone::Neutral,
                        )))
                        .child(
                            view()
                                .w(px(120.0))
                                .flex_row()
                                .items_center()
                                .justify_end()
                                .gap_2()
                                .child(dynamic(move || {
                                    let (share, color_slot) = rows.with(|list| {
                                        list.get(at.index())
                                            .map_or((0.0, 0), |e| (e.share, e.color_slot))
                                    });
                                    ui_treemap_size_bar(share, px(54.0), color_slot).into_element()
                                }))
                                .child(
                                    text(move || {
                                        rows.with(|list| {
                                            list.get(at.index())
                                                .map(|e| format!("{:.1}%", e.share * 100.0))
                                                .unwrap_or_default()
                                        })
                                    })
                                    .w(px(44.0))
                                    .text_xs()
                                    .font_family(font_mono_family())
                                    .tabular_nums()
                                    .text_right()
                                    .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(
                            text(move || {
                                rows.with(|list| {
                                    list.get(at.index())
                                        .map(|e| format_bytes(e.size))
                                        .unwrap_or_default()
                                })
                            })
                            .w(px(88.0))
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
                rows.with(|list| list.iter().map(|c| c.path.clone()).collect::<Vec<String>>())
            }),
        )
}

fn treemap_info_footer(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let hovered = state.treemap_hovered;

    view()
        .h(px(34.0))
        .w_full()
        .px_4()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .bg(move || theme().background)
        .border_t_1()
        .border_color(move || theme().border)
        .child(dynamic(move || match hovered.get() {
            Some(info) => view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .justify_between()
                .gap_4()
                .child(
                    text(format!("/{}", info.path))
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .font_family(font_mono_family())
                        .number_of_lines(1)
                        .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .flex_shrink_0()
                        .child(ui_badge(
                            move || {
                                SharedString::from(info.layer.map_or_else(
                                    || "—".to_owned(),
                                    |l| {
                                        tf(
                                            lang.get(),
                                            "common.layer_number",
                                            &[&(l + 1).to_string()],
                                        )
                                    },
                                ))
                            },
                            BadgeTone::Neutral,
                        ))
                        .child(
                            text(format!(
                                "{} · {:.1}%",
                                format_bytes(info.size),
                                info.share * 100.0
                            ))
                            .text_xs()
                            .font_family(font_mono_family())
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground),
                        ),
                )
                .into_element(),
            None => text(move || t(lang.get(), "treemap.hover_hint"))
                .text_xs()
                .number_of_lines(1)
                .text_color(move || theme().muted_foreground)
                .into_element(),
        }))
}

fn split_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn open_treemap_file(
    state: &AppState,
    res: &DockerAnalyzeResult,
    path: &str,
    layer_idx: Option<usize>,
) {
    let Some(idx) = layer_idx else {
        return;
    };
    let Some(tree) = res.file_tree_list.get(idx) else {
        return;
    };
    let segments = split_segments(path);
    let Some(item) = find_tree_item(tree, &segments) else {
        return;
    };
    let row = FileTreeRow {
        key: path.to_owned(),
        name: item.name.clone(),
        link: item.link.clone(),
        mode: item.mode.clone(),
        uid: item.uid,
        gid: item.gid,
        size: item.size,
        op: item.op.clone(),
        depth: 0,
        is_dir: false,
        expanded: false,
        ratio: 1.0,
    };
    state.selected_layer.set(idx);
    state.open_file_preview(&row);
}

fn find_tree_item<'a>(items: &'a [FileTreeItem], segments: &[String]) -> Option<&'a FileTreeItem> {
    if segments.is_empty() {
        return None;
    }
    let slice = match items {
        [single] if single.name == "." => &single.children,
        _ => items,
    };
    let mut cursor = slice.iter().find(|it| it.name == segments[0])?;
    for seg in &segments[1..] {
        cursor = cursor.children.iter().find(|it| it.name == *seg)?;
    }
    Some(cursor)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use diving_core::{Analyzer, FileTreeItem, ImageLayer, Op};
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::domain::{DivingService, build_final_tree};
    use crate::ui::app_tokens;

    fn large_treemap_result(count: usize) -> Arc<DockerAnalyzeResult> {
        let mut files = Vec::with_capacity(count);
        for i in 0..count {
            files.push(FileTreeItem {
                name: format!("item-{i:02}.dat"),
                size: ((count - i) as u64) * 1024,
                op: Op::Added,
                mode: "-rw-r--r--".to_owned(),
                ..Default::default()
            });
        }
        Arc::new(DockerAnalyzeResult {
            layers: vec![ImageLayer {
                digest: "sha256:tm".to_owned(),
                cmd: "COPY items /".to_owned(),
                size: 1024,
                unpack_size: 102400,
                ..Default::default()
            }],
            file_tree_list: vec![files],
            ..Default::default()
        })
    }

    #[component]
    fn TreemapAsyncAndTableHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let res = large_treemap_result(90);
        state.route.set(AppRoute::Inspector);
        state.active_tab.set(InspectorTab::Treemap);
        state.result.set(Some(res.clone()));

        let load_state = state.clone();
        let load_res = res.clone();
        let preview_sig = state.preview_target;
        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(ui_button(
                        "Resolve tree",
                        ButtonVariant::Ghost,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        move || {
                            let tree = build_final_tree(&load_res.file_tree_list);
                            load_state.final_tree.set(Some(SharedRef::new(tree)));
                        },
                    ))
                    .child(text(move || {
                        preview_sig
                            .get()
                            .map(|t| format!("Preview: {}", t.path))
                            .unwrap_or_default()
                    })),
            )
            .child(treemap_tab(state, res))
    }

    #[test]
    fn treemap_subscribes_to_final_tree_and_recycles_90_table_rows_without_stale_data() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<TreemapAsyncAndTableHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start treemap harness");

        // Starts in loading state while final_tree is None (never blocks UI thread).
        app.wait_for_within("Analyzing…", Duration::from_secs(2))
            .expect("loading indicator rendered while final_tree is None");
        assert!(!app.shows("Root"));

        // Resolving final_tree reactively displays Root & Map/Table toggle.
        app.press("Resolve tree").expect("resolve final_tree");
        app.wait_for_within("Root", Duration::from_secs(2))
            .expect("Root toolbar rendered after final_tree resolved");

        app.press("Table").expect("switch to Table mode");
        app.wait_for_within("item-00.dat", Duration::from_secs(2))
            .expect("top row visible");

        // Scroll virtual_list to the bottom and verify recycled cells show bottom rows accurately.
        let list_id = app
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|n| n.scroll.as_ref().is_some_and(|s| !s.horizontal))
            .expect("virtual_list scroll node")
            .id;
        app.scroll(&format!("view:{list_id}"), px(3000.0))
            .expect("scroll treemap table to bottom");
        app.wait_for_within("item-89.dat", Duration::from_secs(2))
            .expect("bottom row item-89.dat rendered after recycling");
        assert!(
            !app.shows("item-00.dat"),
            "top row item-00.dat should be scrolled out, not leaked into recycled cells"
        );
        app.press("item-89.dat").expect("click recycled bottom row");
        app.wait_for_within("Preview: item-89.dat", Duration::from_secs(2))
            .expect("recycled row on_press must open item-89.dat, not stale item-00.dat");
    }

    #[component]
    fn TreemapShrinkHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let res = large_treemap_result(90);
        let initial = (0..90)
            .map(|i| TreemapTableRow {
                name: format!("item-{i:02}.dat"),
                path: format!("item-{i:02}.dat"),
                size: 1024,
                is_dir: false,
                layer: Some(0),
                share: 1.0 / 90.0,
                color_slot: 1,
            })
            .collect();
        let rows = signal(Arc::new(initial));

        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .child(ui_button(
                "Shrink list",
                ButtonVariant::Ghost,
                UiSize::Sm,
                None,
                false,
                false,
                move || rows.set(Arc::new(Vec::new())),
            ))
            .child(treemap_table_list(state, res, rows))
    }

    #[test]
    fn treemap_table_handles_shrinking_rows_while_scrolled_without_panic() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<TreemapShrinkHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start treemap shrink harness");
        app.wait_for_within("item-00.dat", Duration::from_secs(2))
            .expect("initial rows mounted");
        let list_id = app
            .screen()
            .layout_nodes()
            .into_iter()
            .find(|n| n.scroll.as_ref().is_some_and(|s| !s.horizontal))
            .expect("virtual_list scroll node")
            .id;
        app.scroll(&format!("view:{list_id}"), px(1500.0))
            .expect("scroll down mid-list");
        app.press("Shrink list")
            .expect("shrink list while scrolled must not panic on out-of-bounds index");
        app.wait_for_gone("item-89.dat", Duration::from_secs(2))
            .expect("shrunk list rendered safely");
    }

    #[test]
    fn treemap_table_preserves_layer_slot_colors_and_navigates_with_arrow_keys() {
        use std::collections::BTreeMap;

        let top_layers = vec![2_usize, 0, 5];
        let inner = FsNode {
            name: "bin".to_owned(),
            path: "app/bin".to_owned(),
            size: 600,
            layer: Some(2),
            children: BTreeMap::new(),
        };
        let mut dir_children = BTreeMap::new();
        dir_children.insert("bin".to_owned(), inner);
        let dir_child = FsNode {
            name: "app".to_owned(),
            path: "app".to_owned(),
            size: 600,
            layer: Some(2),
            children: dir_children,
        };
        let other_file = FsNode {
            name: "extra.dat".to_owned(),
            path: "extra.dat".to_owned(),
            size: 400,
            layer: Some(9),
            children: BTreeMap::new(),
        };
        let mut root_children = BTreeMap::new();
        root_children.insert("app".to_owned(), dir_child);
        root_children.insert("extra.dat".to_owned(), other_file);
        let root = FsNode {
            name: String::new(),
            path: String::new(),
            size: 1000,
            layer: None,
            children: root_children,
        };

        let table_rows = treemap_table_rows(&root, &top_layers);
        assert_eq!(table_rows[0].color_slot, 1);
        assert_eq!(table_rows[1].color_slot, 0);

        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<TreemapAsyncAndTableHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start treemap table keyboard harness");
        app.wait_for_within("Analyzing…", Duration::from_secs(2))
            .expect("initial loading indicator visible");
        app.press("Resolve tree").expect("resolve tree");
        app.wait_for_within("Root", Duration::from_secs(2))
            .expect("toolbar visible");
        app.press("Table").expect("switch to Table view");
        app.wait_for_within("item-00.dat", Duration::from_secs(2))
            .expect("table rows rendered");

        // Tab to row 0, then step Down with arrow key and confirm selected state updates.
        for _ in 0..16 {
            app.press_key("tab").expect("tab into treemap table");
            if app.screen().selected("role:button[/item-00\\.dat/]") == Ok(true) {
                break;
            }
        }
        assert_eq!(
            app.screen().selected("role:button[/item-00\\.dat/]"),
            Ok(true)
        );
        app.press_key("down").expect("arrow down in treemap table");
        app.wait_until("second treemap row selected", Duration::from_secs(2), |s| {
            s.selected("role:button[/item-01\\.dat/]") == Ok(true)
        })
        .expect("item-01.dat selected via Down key");
    }
}
