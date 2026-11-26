//! Cross-layer File Provenance search dialog: trace where any file path is added, modified, or deleted across layers.

use std::time::Duration;

use diving_core::Op;
use nsh_core::prelude::*;

use crate::domain::{AppState, FileProvenanceHit, format_bytes, search_file_provenance};
use crate::i18n::t;
use crate::ui::{
    BadgeTone, ButtonVariant, FileOpKind, IconName, UiSize, font_mono_family, radius_inset,
    text_tertiary, theme, ui_badge, ui_button, ui_dialog, ui_empty_state,
    ui_search_input_with_escape, ui_status_badge,
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
                UiSize::Md,
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
    let esc_state = state.clone();

    let debounced_query = signal(query_sig.get_untracked().trim().to_ascii_lowercase());
    let debounce_seq = signal(0_u64);
    effect(move || {
        let normalized = query_sig.get().trim().to_ascii_lowercase();
        let next_seq = debounce_seq.get_untracked().wrapping_add(1);
        debounce_seq.set(next_seq);
        if normalized.is_empty() {
            debounced_query.set(normalized);
        } else {
            set_timeout(Duration::from_millis(120), move || {
                if debounce_seq.try_get() == Some(next_seq) {
                    debounced_query.try_set(normalized);
                }
            });
        }
    });

    let hits: Memo<Vec<FileProvenanceHit>> = computed(move || {
        let q = debounced_query.get();
        if q.is_empty() {
            return Vec::new();
        }
        let Some(res) = res_sig.get() else {
            return Vec::new();
        };
        search_file_provenance(&res, &q, MAX_PROVENANCE_HITS)
    });

    view()
        .w_full()
        .flex_col()
        .gap_3()
        .child(ui_search_input_with_escape(
            query_sig,
            move || t(lang.get(), "provenance.placeholder").to_owned(),
            "Cross-layer file provenance search",
            UiSize::Md,
            None,
            None,
            true,
            None,
            Some(Box::new(move || esc_state.provenance_open.set(false))),
        ))
        .child(scroll().max_h(px(380.0)).w_full().child(dynamic(move || {
            let query = debounced_query.get();
            if query.is_empty() {
                return ui_empty_state(
                    IconName::Search,
                    move || SharedString::from(t(lang.get(), "provenance.title")),
                    move || SharedString::from(t(lang.get(), "provenance.empty")),
                    None,
                )
                .into_element();
            }
            if res_sig.get().is_none() {
                return view().into_element();
            }
            let matched = hits.get();
            if matched.is_empty() {
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
                .p_1()
                .flex_col()
                .gap_1()
                .children(matched.into_iter().map(|hit| {
                    let jump_state = state.clone();
                    let key_jump_state = state.clone();
                    let jump_path = hit.path.clone();
                    let key_jump_path = hit.path.clone();
                    let layer_idx = hit.layer_index;
                    let op_kind = match hit.op {
                        Op::Removed => FileOpKind::Removed,
                        Op::Modified => FileOpKind::Modified,
                        Op::None | Op::Added => FileOpKind::Added,
                    };
                    let layer_badge = format!("#{:02}", layer_idx + 1);
                    let size_str = format_bytes(hit.size);
                    let cmd_preview = hit.layer_cmd;
                    let a11y_label = format!("{layer_badge} {}", hit.path);

                    view()
                        .w_full()
                        .px_2p5()
                        .py_2()
                        .flex_col()
                        .gap_1()
                        .rounded(radius_inset)
                        .border_1()
                        .border_color(move || theme().border)
                        .role(AccessibilityRole::Button)
                        .label(a11y_label)
                        .hover(|s| s.bg(move || theme().accent))
                        .focusable(true)
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_key_down(move |ev: &KeyEvent| {
                            if matches!(ev.key.as_str(), "Enter" | " ") {
                                key_jump_state.jump_to_layer_file(layer_idx, key_jump_path.clone());
                            }
                        })
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
                                    .text_xs()
                                    .font_family(font_mono_family())
                                    .number_of_lines(1)
                                    .text_color(text_tertiary),
                            )
                        })
                }))
                .into_element()
        })))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use diving_core::{Analyzer, DockerAnalyzeResult, FileTreeItem, ImageLayer, Op};
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::domain::DivingService;
    use crate::ui::app_tokens;

    #[component]
    fn ProvenanceHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let res = DockerAnalyzeResult {
            layers: vec![ImageLayer {
                cmd: "COPY etc /etc".to_owned(),
                ..Default::default()
            }],
            file_tree_list: vec![vec![FileTreeItem {
                name: "etc".to_owned(),
                mode: "drwxr-xr-x".to_owned(),
                children: vec![FileTreeItem {
                    name: "app.conf".to_owned(),
                    size: 42,
                    op: Op::Added,
                    mode: "-rw-r--r--".to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            }]],
            ..Default::default()
        };
        state.result.set(Some(Arc::new(res)));
        state.provenance_open.set(true);

        view()
            .flex_1()
            .bg(move || theme().background)
            .child(provenance_dialog(state))
    }

    #[test]
    fn provenance_exposes_button_role_debounces_and_closes_on_two_step_escape() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<ProvenanceHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start provenance harness");
        app.wait_for_within("Cross-Layer File Provenance", Duration::from_secs(2))
            .expect("provenance dialog rendered");

        app.type_text("Cross-layer file provenance search", "app.conf")
            .expect("type provenance query");
        app.wait_for_within("etc/app.conf", Duration::from_secs(2))
            .expect("debounced provenance hit shown");
        assert_eq!(
            app.screen().matches("role:button[#01 etc/app.conf]"),
            Ok(true),
            "expected button role on provenance hit row"
        );

        // 1st Esc clears query, 2nd Esc invokes on_escape to close the dialog while focused.
        app.press_key("escape").expect("1st Esc clears search");
        app.wait_for_gone("etc/app.conf", Duration::from_secs(2))
            .expect("search results cleared");
        assert!(app.shows("Cross-Layer File Provenance"));

        app.press_key("escape").expect("2nd Esc closes dialog");
        app.wait_for_gone("Cross-Layer File Provenance", Duration::from_secs(2))
            .expect("provenance dialog closed on Esc while focused");
    }
}
