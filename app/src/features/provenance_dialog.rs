//! Cross-layer File Provenance search dialog: trace where any file path is added, modified, or deleted across layers.

use diving_core::Op;
use nsh_core::prelude::*;

use crate::domain::{AppState, format_bytes, search_file_provenance};
use crate::i18n::t;
use crate::ui::{
    BadgeTone, ButtonVariant, FileOpKind, IconName, UiSize, font_mono_family, radius_inset,
    text_tertiary, theme, ui_badge, ui_button, ui_dialog, ui_empty_state, ui_search_input,
    ui_status_badge,
};

const MAX_PROVENANCE_HITS: usize = 80;

/// Modal dialog tracing matching file paths across every layer in the loaded image.
pub fn provenance_dialog(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let body_state = state.clone();
    let close_state = state.clone();

    ui_dialog(
        state.provenance_open,
        move || SharedString::from(t(lang.get(), "provenance.title")),
        px(680.0),
        move || provenance_body(body_state.clone()).into_element(),
        move || {
            let s = close_state.clone();
            ui_button(
                move || SharedString::from(t(lang.get(), "common.close")),
                ButtonVariant::Outline,
                UiSize::Sm,
                None,
                false,
                false,
                move || s.provenance_open.set(false),
            )
            .into_element()
        },
    )
}

fn provenance_body(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let query_sig = state.provenance_query;
    let res_sig = state.result;

    view()
        .w_full()
        .flex_col()
        .gap_3()
        .child(ui_search_input(
            query_sig,
            move || t(lang.get(), "provenance.placeholder").to_owned(),
            "Cross-layer file provenance search",
            UiSize::Md,
            None,
            None,
            true,
            None,
        ))
        .child(scroll().max_h(px(380.0)).w_full().child(dynamic(move || {
            let query = query_sig.get();
            if query.trim().is_empty() {
                return ui_empty_state(
                    IconName::Search,
                    move || SharedString::from(t(lang.get(), "provenance.title")),
                    move || SharedString::from(t(lang.get(), "provenance.empty")),
                    None,
                )
                .into_element();
            }
            let Some(res) = res_sig.get() else {
                return view().into_element();
            };
            let hits = search_file_provenance(&res, &query, MAX_PROVENANCE_HITS);
            if hits.is_empty() {
                return ui_empty_state(
                    IconName::Search,
                    move || SharedString::from(t(lang.get(), "provenance.no_matches")),
                    "",
                    None,
                )
                .into_element();
            }

            view()
                .w_full()
                .flex_col()
                .gap_1()
                .children(hits.into_iter().map(|hit| {
                    let jump_state = state.clone();
                    let jump_path = hit.path.clone();
                    let layer_idx = hit.layer_index;
                    let op_kind = match hit.op {
                        Op::Removed => FileOpKind::Removed,
                        Op::Modified => FileOpKind::Modified,
                        Op::None | Op::Added => FileOpKind::Added,
                    };
                    let layer_badge = format!("#{:02}", layer_idx + 1);
                    let size_str = format_bytes(hit.size);
                    let cmd_preview = hit.layer_cmd;

                    view()
                        .w_full()
                        .px_2p5()
                        .py_2()
                        .flex_col()
                        .gap_1()
                        .rounded(radius_inset)
                        .border_1()
                        .border_color(move || theme().border)
                        .hover(|s| s.bg(move || theme().accent))
                        .focusable(true)
                        .on_press(move || {
                            jump_state.jump_to_layer_file(layer_idx, jump_path.clone());
                        })
                        .child(
                            view()
                                .w_full()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(ui_badge(layer_badge, BadgeTone::Neutral))
                                        .child(ui_status_badge(op_kind))
                                        .child(
                                            text(hit.path)
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_xs()
                                                .font_family(font_mono_family())
                                                .number_of_lines(1)
                                                .text_color(move || theme().foreground),
                                        ),
                                )
                                .child(
                                    text(size_str)
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .tabular_nums()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .when(!cmd_preview.is_empty(), |col| {
                            col.child(
                                text(cmd_preview)
                                    .text_size(px(11.0))
                                    .font_family(font_mono_family())
                                    .number_of_lines(1)
                                    .text_color(text_tertiary),
                            )
                        })
                }))
                .into_element()
        })))
}
