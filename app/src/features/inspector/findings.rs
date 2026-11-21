//! Tab 3 (`Findings`): optimization recommendations, wasted space, secrets, duplicates, large files.

use std::sync::Arc;

use diving_core::{DockerAnalyzeResult, DockerAnalyzeSummary};
use nsh_core::prelude::*;

use crate::domain::{AppState, format_bytes};
use crate::i18n::{UiLang, t, tf};
use crate::ui::{
    BadgeTone, IconName, font_mono_family, radius_inset, theme, ui_badge, ui_empty_state,
    ui_panel_titled, ui_separator,
};

fn severity_badge_tone(severity: &str) -> BadgeTone {
    match severity {
        "high" => BadgeTone::Danger,
        "medium" => BadgeTone::Warning,
        "low" => BadgeTone::Info,
        _ => BadgeTone::Neutral,
    }
}

/// Renders all 5 Findings panels in a responsive 2-column (>=1100pt) or 1-column layout.
pub fn findings_tab(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    summary: Arc<DockerAnalyzeSummary>,
) -> impl IntoElement {
    let lang = state.lang;

    scroll().flex_1().min_h(px(0.0)).child(
        view()
            .w_full()
            .p_6()
            .flex_col()
            .gap_6()
            .child(recommendations_panel(res.clone(), lang))
            .child(dynamic(move || {
                if window().width >= px(1100.0) {
                    view()
                        .w_full()
                        .flex_col()
                        .gap_6()
                        .child(
                            view()
                                .w_full()
                                .flex_row()
                                .items_start()
                                .gap_6()
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .child(wasted_panel(summary.clone(), lang)),
                                )
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .child(sensitive_panel(res.clone(), lang)),
                                ),
                        )
                        .child(
                            view()
                                .w_full()
                                .flex_row()
                                .items_start()
                                .gap_6()
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .child(duplicates_panel(res.clone(), lang)),
                                )
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .child(big_files_panel(res.clone(), lang)),
                                ),
                        )
                        .into_element()
                } else {
                    view()
                        .w_full()
                        .flex_col()
                        .gap_6()
                        .child(wasted_panel(summary.clone(), lang))
                        .child(sensitive_panel(res.clone(), lang))
                        .child(duplicates_panel(res.clone(), lang))
                        .child(big_files_panel(res.clone(), lang))
                        .into_element()
                }
            })),
    )
}

fn recommendations_panel(res: Arc<DockerAnalyzeResult>, lang: Signal<UiLang>) -> impl IntoElement {
    let count = res.recommendations.len();
    ui_panel_titled(
        move || {
            SharedString::from(format!(
                "{} ({count})",
                t(lang.get(), "findings.recommendations")
            ))
        },
        None,
        if res.recommendations.is_empty() {
            ui_empty_state(
                IconName::Shield,
                move || SharedString::from(t(lang.get(), "findings.recommendations_empty")),
                "",
                None,
            )
            .into_element()
        } else {
            view()
                .flex_col()
                .gap_3()
                .children(res.recommendations.iter().enumerate().map(|(idx, rec)| {
                    let tone = severity_badge_tone(&rec.severity);
                    let severity = rec.severity.clone();
                    let category = rec.category.clone();
                    let title = rec.title.clone();
                    let detail = rec.detail.clone();
                    let hint = rec.dockerfile_hint.clone();
                    let paths = rec.paths.clone();
                    let est_saved = rec.est_saved_bytes;

                    view()
                        .flex_col()
                        .gap_2()
                        .when(idx > 0, |v| {
                            v.pt_3().border_t_1().border_color(move || theme().border)
                        })
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
                                        .child(ui_badge(severity, tone))
                                        .child(ui_badge(category, BadgeTone::Neutral))
                                        .child(
                                            text(title)
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().foreground),
                                        ),
                                )
                                .when(est_saved > 0, |r| {
                                    r.child(ui_badge(
                                        move || {
                                            SharedString::from(tf(
                                                lang.get(),
                                                "findings.est_savings",
                                                &[&format_bytes(est_saved)],
                                            ))
                                        },
                                        BadgeTone::Info,
                                    ))
                                }),
                        )
                        .child(
                            text(detail)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .when(!hint.is_empty(), |col| {
                            col.child(
                                view()
                                    .p_2p5()
                                    .rounded(radius_inset)
                                    .bg(move || theme().muted)
                                    .border_1()
                                    .border_color(move || theme().border)
                                    .child(
                                        text(hint)
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .text_color(move || theme().foreground),
                                    ),
                            )
                        })
                        .when(!paths.is_empty(), |col| {
                            col.child(view().flex_row().flex_wrap().gap_1p5().children(
                                paths.into_iter().map(|p| ui_badge(p, BadgeTone::Neutral)),
                            ))
                        })
                }))
                .into_element()
        },
    )
}

fn wasted_panel(summary: Arc<DockerAnalyzeSummary>, lang: Signal<UiLang>) -> impl IntoElement {
    let count = summary.wasted_list.len();
    ui_panel_titled(
        move || SharedString::from(format!("{} ({count})", t(lang.get(), "findings.wasted"))),
        Some(
            ui_badge(
                format_bytes(summary.wasted_size),
                if summary.wasted_size > 0 {
                    BadgeTone::Warning
                } else {
                    BadgeTone::Success
                },
            )
            .into_element(),
        ),
        if summary.wasted_list.is_empty() {
            ui_empty_state(
                IconName::Check,
                move || SharedString::from(t(lang.get(), "findings.wasted_empty")),
                "",
                None,
            )
            .into_element()
        } else {
            view()
                .flex_col()
                .gap_2()
                .children(
                    summary
                        .wasted_list
                        .iter()
                        .take(40)
                        .enumerate()
                        .map(|(idx, item)| {
                            let path = item.path.clone();
                            let size = format_bytes(item.total_size);
                            let changes_desc: Vec<String> = item
                                .changes
                                .iter()
                                .map(|c| {
                                    let to_layer = (c.layer + 1).to_string();
                                    match (c.written_in, c.removed) {
                                        (Some(w), true) => tf(
                                            lang.get_untracked(),
                                            "findings.waste_removed_from",
                                            &[&(w + 1).to_string(), &to_layer],
                                        ),
                                        (Some(w), false) => tf(
                                            lang.get_untracked(),
                                            "findings.waste_overwritten_from",
                                            &[&(w + 1).to_string(), &to_layer],
                                        ),
                                        (None, true) => tf(
                                            lang.get_untracked(),
                                            "findings.waste_removed_in",
                                            &[&to_layer],
                                        ),
                                        (None, false) => tf(
                                            lang.get_untracked(),
                                            "findings.waste_overwritten_in",
                                            &[&to_layer],
                                        ),
                                    }
                                })
                                .collect();

                            view()
                                .flex_col()
                                .gap_1()
                                .when(idx > 0, |v| v.pt_2().child(ui_separator()))
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            text(path)
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_xs()
                                                .font_family(font_mono_family())
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(size)
                                                .text_xs()
                                                .font_family(font_mono_family())
                                                .tabular_nums()
                                                .text_color(move || theme().foreground),
                                        ),
                                )
                                .child(
                                    text(changes_desc.join(" · "))
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                )
                        }),
                )
                .into_element()
        },
    )
}

fn sensitive_panel(res: Arc<DockerAnalyzeResult>, lang: Signal<UiLang>) -> impl IntoElement {
    let count = res.sensitive_files.len();
    ui_panel_titled(
        move || SharedString::from(format!("{} ({count})", t(lang.get(), "findings.sensitive"))),
        None,
        if res.sensitive_files.is_empty() {
            ui_empty_state(
                IconName::Shield,
                move || SharedString::from(t(lang.get(), "findings.sensitive_empty")),
                "",
                None,
            )
            .into_element()
        } else {
            view()
                .flex_col()
                .gap_2()
                .children(res.sensitive_files.iter().enumerate().map(|(idx, item)| {
                    let path = item.path.clone();
                    let reason = item.reason.clone();
                    let layer = format!("L{}", item.layer_index + 1);
                    let size = format_bytes(item.size);
                    view()
                        .flex_col()
                        .gap_1()
                        .when(idx > 0, |v| v.pt_2().child(ui_separator()))
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    text(path)
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .text_color(move || theme().foreground),
                                )
                                .child(ui_badge(layer, BadgeTone::Warning))
                                .child(
                                    text(size)
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .tabular_nums()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(
                            text(reason)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                }))
                .into_element()
        },
    )
}

fn duplicates_panel(res: Arc<DockerAnalyzeResult>, lang: Signal<UiLang>) -> impl IntoElement {
    let count = res.duplicate_groups.len();
    ui_panel_titled(
        move || {
            SharedString::from(format!(
                "{} ({count})",
                t(lang.get(), "findings.duplicates")
            ))
        },
        None,
        if res.duplicate_groups.is_empty() {
            ui_empty_state(
                IconName::Check,
                move || SharedString::from(t(lang.get(), "findings.duplicates_empty")),
                "",
                None,
            )
            .into_element()
        } else {
            view()
                .flex_col()
                .gap_2()
                .children(
                    res.duplicate_groups
                        .iter()
                        .take(30)
                        .enumerate()
                        .map(|(idx, grp)| {
                            let first_path = grp
                                .paths
                                .first()
                                .map(|p| p.path.clone())
                                .unwrap_or_else(|| grp.hash.clone());
                            let copies = grp.count.to_string();
                            let wasted = format_bytes(grp.total_wasted);
                            let meta = tf(
                                lang.get_untracked(),
                                "findings.duplicates_copies",
                                &[&copies, &wasted],
                            );
                            view()
                                .flex_col()
                                .gap_1()
                                .when(idx > 0, |v| v.pt_2().child(ui_separator()))
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            text(first_path)
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_xs()
                                                .font_family(font_mono_family())
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(format_bytes(grp.size))
                                                .text_xs()
                                                .font_family(font_mono_family())
                                                .tabular_nums()
                                                .text_color(move || theme().muted_foreground),
                                        ),
                                )
                                .child(
                                    text(meta)
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                )
                        }),
                )
                .into_element()
        },
    )
}

fn big_files_panel(res: Arc<DockerAnalyzeResult>, lang: Signal<UiLang>) -> impl IntoElement {
    let count = res.big_modified_file_list.len();
    ui_panel_titled(
        move || SharedString::from(format!("{} ({count})", t(lang.get(), "findings.big_files"))),
        None,
        if res.big_modified_file_list.is_empty() {
            ui_empty_state(
                IconName::Check,
                move || SharedString::from(t(lang.get(), "findings.big_files_empty")),
                "",
                None,
            )
            .into_element()
        } else {
            view()
                .flex_col()
                .gap_2()
                .children(res.big_modified_file_list.iter().take(40).enumerate().map(
                    |(idx, item)| {
                        let path = item.path.clone();
                        let size = format_bytes(item.size);
                        let mode = item.mode.clone();
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .when(idx > 0, |v| {
                                v.pt_2().border_t_1().border_color(move || theme().border)
                            })
                            .child(
                                text(path)
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .font_family(font_mono_family())
                                    .text_color(move || theme().foreground),
                            )
                            .when(!mode.is_empty(), |r| {
                                r.child(ui_badge(mode, BadgeTone::Neutral))
                            })
                            .child(
                                text(size)
                                    .text_xs()
                                    .font_family(font_mono_family())
                                    .tabular_nums()
                                    .text_color(move || theme().foreground),
                            )
                    },
                ))
                .into_element()
        },
    )
}
