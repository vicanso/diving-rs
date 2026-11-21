//! Left master pane (`LayerTimeline`) with 5 alignment spines and right-click context menu.

use std::sync::Arc;

use diving_core::DockerAnalyzeResult;
use nsh_core::prelude::*;

use crate::domain::{AppState, LayerFilterMode, classify_layer_opcode, format_bytes};
use crate::i18n::t;
use crate::ui::{
    IconName, SegmentOption, UiSize, font_mono_family, radius_inset, surface_active, text_tertiary,
    theme, ui_context_menu, ui_empty_state, ui_menu_item, ui_opcode_pill, ui_segmented,
    ui_size_bar,
};

const LARGE_LAYER_BYTES: u64 = 1_048_576;

/// Renders the master `LayerTimeline` sidebar panel.
pub fn layer_timeline_panel(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let lang = state.lang;
    let layer_filter = state.layer_filter;
    let selected_layer = state.selected_layer;

    let layer_count = res.layers.len();
    let total_compressed: u64 = res.layers.iter().map(|l| l.size).sum();
    let max_size = res.layers.iter().map(|l| l.size).max().unwrap_or(1).max(1);

    let all_entries: Vec<(usize, _)> = res.layers.iter().cloned().enumerate().collect();

    let filtered_entries = computed(move || {
        let mode = layer_filter.get();
        all_entries
            .iter()
            .filter(|(_, layer)| match mode {
                LayerFilterMode::All => true,
                LayerFilterMode::HasFiles => !layer.empty,
                LayerFilterMode::Large => layer.size > LARGE_LAYER_BYTES,
            })
            .cloned()
            .collect::<Vec<_>>()
    });

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_col()
        .bg(move || theme().sidebar)
        .child(
            view()
                .flex_col()
                .gap_2()
                .p_3()
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            text(move || t(lang.get(), "layers.title"))
                                .role(AccessibilityRole::Header)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().sidebar_foreground),
                        )
                        .child(
                            text(move || {
                                format!(
                                    "{}/{layer_count} · {}",
                                    filtered_entries.get().len(),
                                    format_bytes(total_compressed)
                                )
                            })
                            .text_xs()
                            .font_family(font_mono_family())
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(ui_segmented(
                    vec![
                        SegmentOption::new(LayerFilterMode::All, move || {
                            SharedString::from(t(lang.get(), "layers.filter_all"))
                        }),
                        SegmentOption::new(LayerFilterMode::HasFiles, move || {
                            SharedString::from(t(lang.get(), "layers.filter_files"))
                        }),
                        SegmentOption::new(LayerFilterMode::Large, move || {
                            SharedString::from(t(lang.get(), "layers.filter_large"))
                        }),
                    ],
                    layer_filter,
                    UiSize::Sm,
                    "Filter image layers",
                    move |mode| layer_filter.set(mode),
                )),
        )
        .child(
            scroll().flex_1().min_h(px(0.0)).child(
                view()
                    .p_2()
                    .gap_0p5()
                    .flex_col()
                    .child(when(
                        move || filtered_entries.get().is_empty(),
                        move || {
                            ui_empty_state(
                                IconName::Layers,
                                move || SharedString::from(t(lang.get(), "layers.no_match")),
                                "",
                                None,
                            )
                        },
                    ))
                    .child(each(
                        move || filtered_entries.get(),
                        |(orig_idx, _)| *orig_idx,
                        move |item, _| {
                            let click_state = state.clone();
                            let (orig_idx, layer) = item.get_untracked();
                            let (opcode, summary) = classify_layer_opcode(&layer.cmd);
                            let ratio =
                                ((layer.size as f64) / (max_size as f64)).clamp(0.0, 1.0) as f32;
                            let is_zero = layer.size == 0;
                            let size_label = format_bytes(layer.size);
                            let cmd_for_copy = layer.cmd.clone();
                            let digest_for_copy = layer.digest.clone();

                            let row = view()
                                .w_full()
                                .min_h(px(32.0))
                                .px_2()
                                .py_1()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .rounded(radius_inset)
                                .overflow_hidden()
                                .bg(move || {
                                    if selected_layer.get() == orig_idx {
                                        surface_active()
                                    } else {
                                        theme().sidebar
                                    }
                                })
                                .hover(|s| s.bg(move || theme().sidebar_accent))
                                .on_press(move || click_state.select_layer(orig_idx))
                                .child(
                                    text(format!("{:02}", orig_idx + 1))
                                        .w(px(20.0))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .tabular_nums()
                                        .text_right()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(ui_opcode_pill(opcode))
                                .child(
                                    text(size_label)
                                        .w(px(58.0))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .tabular_nums()
                                        .text_right()
                                        .text_color(move || {
                                            if is_zero {
                                                text_tertiary()
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                )
                                .child(ui_size_bar(ratio, px(36.0)))
                                .child(
                                    text(summary)
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_xs()
                                        .number_of_lines(1)
                                        .text_color(move || theme().sidebar_foreground),
                                );

                            ui_context_menu(row, |_| {})
                                .child(ui_menu_item(
                                    move || {
                                        SharedString::from(t(lang.get(), "layers.ctx_copy_cmd"))
                                    },
                                    Some(IconName::Copy),
                                    None,
                                    false,
                                    move || set_clipboard(cmd_for_copy.clone()),
                                ))
                                .child(ui_menu_item(
                                    move || {
                                        SharedString::from(t(lang.get(), "layers.ctx_copy_digest"))
                                    },
                                    Some(IconName::Copy),
                                    None,
                                    false,
                                    move || set_clipboard(digest_for_copy.clone()),
                                ))
                        },
                    )),
            ),
        )
}
