//! Left master pane (`LayerTimeline`) with 5 alignment spines and right-click context menu.

use std::sync::Arc;

use diving_core::DockerAnalyzeResult;
use nsh_core::prelude::*;

use crate::domain::{
    AppState, LayerFilterMode, LayerSortOrder, classify_layer_opcode, format_bytes,
    layer_matches_filter,
};
use crate::i18n::t;
use crate::ui::{
    ButtonVariant, IconName, SegmentOption, UiSize, font_mono_family, radius_inset, surface_active,
    text_tertiary, theme, ui_button, ui_context_menu, ui_empty_state, ui_menu_item, ui_opcode_pill,
    ui_segmented, ui_size_bar, ui_tooltip,
};

/// Renders the master `LayerTimeline` sidebar panel.
pub fn layer_timeline_panel(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let lang = state.lang;
    let layer_filter = state.layer_filter;
    let layer_order = state.layer_order;
    let selected_layer = state.selected_layer;
    let sort_state = state.clone();

    let layer_count = res.layers.len();
    let total_compressed: u64 = res.layers.iter().map(|l| l.size).sum();
    let max_size = res.layers.iter().map(|l| l.size).max().unwrap_or(1).max(1);

    let all_entries: Vec<(usize, _)> = res.layers.iter().cloned().enumerate().collect();

    let filtered_entries = computed(move || {
        let mode = layer_filter.get();
        let order = layer_order.get();
        let mut items: Vec<_> = all_entries
            .iter()
            .filter(|(_, layer)| layer_matches_filter(layer, mode))
            .cloned()
            .collect();
        if order == LayerSortOrder::Desc {
            items.reverse();
        }
        items
    });

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_col()
        .bg(move || theme().background)
        .child(
            view()
                .flex_col()
                .gap_2()
                .px_3()
                .py_2p5()
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_1p5()
                        .child(
                            text(move || t(lang.get(), "layers.title"))
                                .role(AccessibilityRole::Header)
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
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
                                )
                                .child(
                                    view()
                                        .tooltip(ui_tooltip(move || {
                                            let key = if layer_order.get() == LayerSortOrder::Desc {
                                                "layers.order_desc_tip"
                                            } else {
                                                "layers.order_asc_tip"
                                            };
                                            SharedString::from(t(lang.get(), key))
                                        }))
                                        .child(ui_button(
                                            move || {
                                                let key =
                                                    if layer_order.get() == LayerSortOrder::Desc {
                                                        "layers.order_desc"
                                                    } else {
                                                        "layers.order_asc"
                                                    };
                                                SharedString::from(t(lang.get(), key))
                                            },
                                            ButtonVariant::Ghost,
                                            UiSize::Xs,
                                            Some(IconName::SortOrder),
                                            false,
                                            false,
                                            move || sort_state.toggle_layer_order(),
                                        )),
                                ),
                        ),
                )
                .child(view().self_start().child(ui_segmented(
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
                ))),
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

                            let key_state = click_state.clone();
                            let row = view()
                                .w_full()
                                .min_h(px(30.0))
                                .px_2()
                                .py_1()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .rounded(radius_inset)
                                .overflow_hidden()
                                .focusable(true)
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .bg(move || {
                                    if selected_layer.get() == orig_idx {
                                        surface_active()
                                    } else {
                                        theme().background
                                    }
                                })
                                .hover(|s| s.bg(move || theme().accent))
                                .on_key_down(move |ev: &KeyEvent| {
                                    if matches!(ev.key.as_str(), "Enter" | " ") {
                                        key_state.select_layer(orig_idx);
                                    }
                                })
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
                                        .w(px(66.0))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .tabular_nums()
                                        .number_of_lines(1)
                                        .text_right()
                                        .text_color(move || {
                                            if is_zero {
                                                text_tertiary()
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                )
                                .child(ui_size_bar(ratio, px(22.0)))
                                .child(
                                    text(summary)
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_xs()
                                        .number_of_lines(1)
                                        .text_color(move || theme().foreground),
                                );

                            let digest_empty = digest_for_copy.is_empty();
                            ui_context_menu(row, |_| {})
                                .child(ui_menu_item(
                                    move || {
                                        SharedString::from(t(lang.get(), "layers.ctx_copy_cmd"))
                                    },
                                    Some(IconName::Copy),
                                    None,
                                    false,
                                    move || {
                                        drop(set_clipboard(cmd_for_copy.clone()));
                                    },
                                ))
                                .child(ui_menu_item(
                                    move || {
                                        SharedString::from(t(lang.get(), "layers.ctx_copy_digest"))
                                    },
                                    Some(IconName::Copy),
                                    None,
                                    digest_empty,
                                    move || {
                                        if !digest_for_copy.is_empty() {
                                            drop(set_clipboard(digest_for_copy.clone()));
                                        }
                                    },
                                ))
                        },
                    )),
            ),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use diving_core::{Analyzer, ImageLayer};
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::domain::DivingService;
    use crate::ui::app_tokens;

    #[component]
    fn LayerTimelineHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let res = DockerAnalyzeResult {
            layers: vec![ImageLayer {
                digest: String::new(),
                cmd: "ENV A=1".to_owned(),
                size: 0,
                unpack_size: 0,
                empty: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        view()
            .w(px(340.0))
            .h(px(480.0))
            .bg(move || theme().background)
            .child(layer_timeline_panel(state, Arc::new(res)))
    }

    #[test]
    fn layer_timeline_segmented_filter_stays_compact() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(960.0), Points(640.0)),
            ..EngineConfig::new(windowed_root::<LayerTimelineHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start layer_timeline harness");
        app.wait_for_within("Layers", Duration::from_secs(2))
            .expect("layer timeline rendered");
        let nodes = app.screen().layout_nodes();
        let all_node = nodes
            .iter()
            .find(|n| n.text.as_ref().is_some_and(|t| t.text.text == "All"))
            .expect("All segment text node");
        let large_node = nodes
            .iter()
            .find(|n| n.text.as_ref().is_some_and(|t| t.text.text == "> 1 MB"))
            .expect("> 1 MB segment text node");
        let mut cursor_parent = all_node.parent;
        let mut segmented_width = 0.0_f32;
        while let Some(node) = nodes.iter().find(|n| n.id == cursor_parent) {
            let b = node.bounds;
            if b.origin.x.0 <= all_node.bounds.origin.x.0
                && b.origin.x.0 + b.size.width.0
                    >= large_node.bounds.origin.x.0 + large_node.bounds.size.width.0
            {
                segmented_width = b.size.width.0;
                break;
            }
            if node.parent == node.id {
                break;
            }
            cursor_parent = node.parent;
        }
        // Segmented track fits compactly inside 340pt panel without stretching full width (316pt).
        assert!(
            segmented_width > 60.0 && segmented_width < 260.0,
            "expected compact segmented track width, got {segmented_width}"
        );
    }

    #[component]
    fn TwoLayerTimelineHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let res = DockerAnalyzeResult {
            layers: vec![
                ImageLayer {
                    digest: "sha256:base".to_owned(),
                    cmd: "FROM alpine".to_owned(),
                    size: 1024,
                    unpack_size: 2048,
                    empty: false,
                    ..Default::default()
                },
                ImageLayer {
                    digest: "sha256:app".to_owned(),
                    cmd: "COPY app /app".to_owned(),
                    size: 4096,
                    unpack_size: 8192,
                    empty: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        view()
            .w(px(340.0))
            .h(px(480.0))
            .bg(move || theme().background)
            .child(layer_timeline_panel(state, Arc::new(res)))
    }

    #[test]
    fn layer_timeline_defaults_to_reverse_order_and_toggles_order() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(960.0), Points(640.0)),
            ..EngineConfig::new(windowed_root::<TwoLayerTimelineHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start two-layer timeline harness");
        app.wait_for_within("#N→1", Duration::from_secs(2))
            .expect("default reverse sort button shown");

        let row_y = |app: &Headless, badge: &str| -> f32 {
            app.screen()
                .layout_nodes()
                .into_iter()
                .find(|n| n.text.as_ref().is_some_and(|t| t.text.text == badge))
                .map(|n| n.bounds.origin.y.0)
                .unwrap_or_else(|| panic!("missing badge {badge}"))
        };

        assert!(
            row_y(&app, "02") < row_y(&app, "01"),
            "layer 02 should appear above layer 01 by default"
        );

        app.press("#N→1").expect("toggle to build order");
        app.wait_for_within("#1→N", Duration::from_secs(2))
            .expect("toggle label updated to #1→N");
        assert!(
            row_y(&app, "01") < row_y(&app, "02"),
            "layer 01 should appear above layer 02 after Asc toggle"
        );

        app.press("#1→N").expect("restore default reverse order");
        app.wait_for_within("#N→1", Duration::from_secs(2))
            .expect("restored #N→1");
    }
}
