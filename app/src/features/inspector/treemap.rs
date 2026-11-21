//! Tab 2 (`Treemap`): final-filesystem squarified spatial map, breadcrumbs, layer legend, and Table view.

use std::sync::Arc;

use diving_core::{DockerAnalyzeResult, FileTreeItem};
use nsh_core::prelude::*;

use crate::domain::{
    AppState, FileTreeRow, FsNode, SharedRef, TreemapHoverInfo, TreemapTile, TreemapViewMode,
    build_final_tree, dominant_layer, find_fs_node, format_bytes, layout_treemap_tiles,
    top_three_layers,
};
use crate::features::sidebar::APP_SIDEBAR_WIDTH;
use crate::i18n::{t, tf};
use crate::ui::{
    BadgeTone, ButtonVariant, IconName, SegmentOption, UiSize, font_mono_family, radius_control,
    radius_inset, theme, treemap_ink, treemap_slot, ui_badge, ui_button, ui_empty_state, ui_icon,
    ui_segmented, ui_size_bar,
};

const MIN_LABEL_W: f32 = 56.0;
const MIN_LABEL_H: f32 = 24.0;
const MIN_SIZE_LABEL_W: f32 = 74.0;
const MIN_SIZE_LABEL_H: f32 = 38.0;

/// Renders the `Treemap` workspace tab with Map/Table toggle, drill-down breadcrumbs, and Top-3 layer legend.
pub fn treemap_tab(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let root_node = state
        .final_tree
        .get_untracked()
        .unwrap_or_else(|| SharedRef::new(build_final_tree(&res.file_tree_list)));
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
        .child(treemap_body(state.clone(), res, root_node, top_layers))
        .child(treemap_info_footer(state))
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
    let canvas_size = computed(move || {
        let win = window();
        let app_sidebar_w = if size_state.app_sidebar_open.get() {
            APP_SIDEBAR_WIDTH
        } else {
            0.0
        };
        let raw_w = (win.width.0 - app_sidebar_w - size_state.sidebar_width.get().0 - 40.0)
            .clamp(320.0, 1280.0);
        let raw_h = (win.height.0 - 216.0).clamp(260.0, 680.0);
        let w = ((raw_w / 24.0).floor() * 24.0).max(320.0) as u32;
        let h = ((raw_h / 24.0).floor() * 24.0).max(260.0) as u32;
        (w, h)
    });

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .flex_col()
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
                    treemap_canvas_view(
                        state.clone(),
                        res.clone(),
                        current,
                        &top_layer_ids,
                        w as f32,
                        h as f32,
                    )
                    .into_element()
                }
                TreemapViewMode::Table => {
                    treemap_table_view(state.clone(), res.clone(), current).into_element()
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
                                .text_color(move || treemap_ink(slot == 0).opacity(0.85)),
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
                .bg(move || treemap_slot(sub_slot).opacity(0.84))
                .border_1()
                .border_color(move || theme().background.opacity(0.75))
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
                                            treemap_ink(sub_slot == 0).opacity(0.85)
                                        }),
                                )
                            }),
                    )
                })
        }))
}

fn treemap_table_view(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    node: &FsNode,
) -> impl IntoElement {
    let lang = state.lang;
    let total_size = node.size.max(1);
    let mut children: Vec<FsNode> = node
        .children
        .values()
        .filter(|c| c.size > 0)
        .cloned()
        .collect();
    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    let items = Arc::new(children);

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
                {
                    let items = Arc::clone(&items);
                    move || items.len()
                },
                {
                    let items = Arc::clone(&items);
                    let state = state.clone();
                    let res = res.clone();
                    move |at| {
                        let entry = items[at.index()].clone();
                        let click_state = state.clone();
                        let click_res = res.clone();
                        let hover_state = state.clone();
                        let is_dir = entry.is_dir();
                        let dom = dominant_layer(&entry);
                        let layer = dom.map(|(l, _)| l).or(entry.layer);
                        let share = (entry.size as f64 / total_size as f64) as f32;
                        let pct_label = format!("{:.1}%", share * 100.0);
                        let path_clone = entry.path.clone();
                        let hover_info = TreemapHoverInfo {
                            name: entry.name.clone(),
                            path: entry.path.clone(),
                            size: entry.size,
                            layer,
                            share,
                        };

                        view()
                            .w_full()
                            .min_h(px(32.0))
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .border_b_1()
                            .border_color(move || theme().border.opacity(0.45))
                            .hover(|s| s.bg(move || theme().accent))
                            .on_mouse_enter(move |_: &MouseEvent| {
                                hover_state.treemap_hovered.set(Some(hover_info.clone()));
                            })
                            .on_press(move || {
                                if is_dir {
                                    click_state.treemap_path.set(split_segments(&path_clone));
                                    click_state.treemap_hovered.set(None);
                                } else {
                                    open_treemap_file(&click_state, &click_res, &path_clone, layer);
                                }
                            })
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(ui_icon(
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
                                    ))
                                    .child(
                                        text(entry.name)
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .number_of_lines(1)
                                            .text_color(move || theme().foreground),
                                    ),
                            )
                            .child(view().w(px(72.0)).items_start().child(ui_badge(
                                layer.map_or_else(|| "—".to_owned(), |l| format!("L{}", l + 1)),
                                BadgeTone::Neutral,
                            )))
                            .child(
                                view()
                                    .w(px(120.0))
                                    .flex_row()
                                    .items_center()
                                    .justify_end()
                                    .gap_2()
                                    .child(ui_size_bar(share, px(54.0)))
                                    .child(
                                        text(pct_label)
                                            .w(px(44.0))
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .tabular_nums()
                                            .text_right()
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            )
                            .child(
                                text(format_bytes(entry.size))
                                    .w(px(88.0))
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
                items
                    .iter()
                    .map(|c| c.path.clone())
                    .collect::<Vec<String>>()
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
