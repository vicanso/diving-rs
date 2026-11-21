//! Main Image Inspector workspace: single-line metric ribbon, LayerTimeline, detail canvas, and status bar.

pub mod dockerfile;
pub mod file_tree;
pub mod findings;
pub mod layer_timeline;
pub mod treemap;

use std::sync::Arc;

use diving_core::{DockerAnalyzeResult, DockerAnalyzeSummary};
use nsh_core::prelude::*;

use crate::domain::{
    AppState, InspectorTab, format_bytes, format_runtime_compat, image_source_kind,
};
use crate::i18n::{UiLang, t, tf};
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, UiSize, danger, font_mono_family, success,
    text_tertiary, theme, ui_badge, ui_banner, ui_button, ui_empty_state, ui_icon,
    ui_resizable_split, warning,
};

/// Renders the full Inspector view once an analysis result is present.
pub fn inspector_workspace(state: AppState) -> impl IntoElement {
    let lang = state.lang;

    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .child(dynamic(move || {
            let nav_state = state.clone();
            let Some(res) = state.result.get() else {
                return view()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(ui_empty_state(
                        IconName::Layers,
                        move || SharedString::from(t(lang.get(), "inspector.empty_title")),
                        move || SharedString::from(t(lang.get(), "inspector.empty_desc")),
                        Some(
                            ui_button(
                                move || SharedString::from(t(lang.get(), "nav.back_home")),
                                ButtonVariant::Primary,
                                UiSize::Md,
                                None,
                                false,
                                false,
                                move || nav_state.navigate_home(),
                            )
                            .into_element(),
                        ),
                    ))
                    .into_element();
            };
            let Some(summary) = state.summary.get() else {
                return view().into_element();
            };
            let summary = summary.0;

            let retry_state = state.clone();
            let split_state = state.clone();
            let resize_state = state.clone();
            let compact = window().width < px(1100.0);
            let left_max = if compact { px(320.0) } else { px(480.0) };
            let left_initial = px(state
                .sidebar_width
                .get_untracked()
                .0
                .clamp(260.0, left_max.0));

            view()
                .flex_1()
                .min_h(px(0.0))
                .flex_col()
                .when_some(res.stale_as_of.clone(), |col, stale_time| {
                    let s = retry_state.clone();
                    col.child(
                        view().px_4().pt_2().child(ui_banner(
                            BannerTone::Warning,
                            move || {
                                SharedString::from(tf(lang.get(), "stale.banner", &[&stale_time]))
                            },
                            Some(
                                ui_button(
                                    move || SharedString::from(t(lang.get(), "landing.retry")),
                                    ButtonVariant::Outline,
                                    UiSize::Xs,
                                    Some(IconName::Refresh),
                                    false,
                                    false,
                                    move || s.submit_current_input(),
                                )
                                .into_element(),
                            ),
                        )),
                    )
                })
                .when_some(state.analyze_error.get(), |col, err| {
                    col.child(
                        view()
                            .px_4()
                            .pt_2()
                            .child(ui_banner(BannerTone::Danger, err, None)),
                    )
                })
                .child(metric_ribbon(&res, &summary, lang))
                .child(view().flex_1().min_h(px(0.0)).child(ui_resizable_split(
                    left_initial,
                    px(260.0),
                    left_max,
                    layer_timeline::layer_timeline_panel(split_state.clone(), res.clone()),
                    right_workspace(split_state, res.clone(), summary),
                    move |sizes: &Vec<Points>| {
                        if let Some(first) = sizes.first().copied() {
                            resize_state.set_sidebar_width(first);
                        }
                    },
                )))
                .child(bottom_status_bar(state.clone()))
                .into_element()
        }))
}

fn metric_ribbon(
    res: &Arc<DockerAnalyzeResult>,
    summary: &Arc<DockerAnalyzeSummary>,
    lang: Signal<UiLang>,
) -> impl IntoElement {
    let score = summary.score;
    let score_icon = if score >= 90 {
        IconName::Check
    } else {
        IconName::Alert
    };
    let score_color = move || {
        if score >= 95 {
            success()
        } else if score >= 85 {
            warning()
        } else {
            danger()
        }
    };
    let wasted_size = summary.wasted_size;
    let wasted_pct = format!("{:.1}%", summary.wasted_percent * 100.0);
    let has_waste = wasted_size > 0;
    let platform = format!("{}/{}", res.os, res.arch);
    let base_os = res.base_os.clone();
    let user = res.user.clone();
    let runtime_compat = res.runtime_compat.clone();
    let runtime_tone = format_runtime_compat(&runtime_compat, UiLang::En).map(|(_, tone)| tone);
    let tags: Vec<String> = res
        .tags
        .iter()
        .take(3)
        .map(|tag| tag.trim_matches(|c| c == '[' || c == ']').to_owned())
        .collect();
    let overflow_tags = res.tags.len().saturating_sub(3);

    view()
        .w_full()
        .min_h(px(36.0))
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_3()
        .px_4()
        .py_1p5()
        .bg(move || theme().background)
        .border_b_1()
        .border_color(move || theme().border)
        .child(metric_pair(
            move || SharedString::from(t(lang.get(), "kpi.efficiency")),
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(ui_icon(score_icon, px(13.0), score_color))
                .child(
                    text(format!("{score}%"))
                        .text_xs()
                        .font_family(font_mono_family())
                        .font_weight(FontWeight::SEMIBOLD)
                        .tabular_nums()
                        .text_color(score_color),
                ),
        ))
        .child(metric_divider())
        .child(metric_mono_pair(
            move || SharedString::from(t(lang.get(), "kpi.image_size")),
            format_bytes(res.total_size),
        ))
        .child(metric_divider())
        .child(metric_mono_pair(
            move || SharedString::from(t(lang.get(), "kpi.compressed")),
            format_bytes(res.size),
        ))
        .child(metric_divider())
        .child(metric_pair(
            move || SharedString::from(t(lang.get(), "kpi.wasted")),
            text(if has_waste {
                format!("{} ({wasted_pct})", format_bytes(wasted_size))
            } else {
                "0 B".to_owned()
            })
            .text_xs()
            .font_family(font_mono_family())
            .font_weight(FontWeight::MEDIUM)
            .tabular_nums()
            .text_color(move || {
                if has_waste {
                    warning()
                } else {
                    theme().foreground
                }
            }),
        ))
        .child(metric_divider())
        .child(metric_mono_pair(
            move || SharedString::from(t(lang.get(), "kpi.platform")),
            platform,
        ))
        .when(!base_os.is_empty(), |row| {
            row.child(metric_divider()).child(metric_mono_pair(
                move || SharedString::from(t(lang.get(), "kpi.base_os")),
                base_os,
            ))
        })
        .when(!user.is_empty(), |row| {
            row.child(metric_divider()).child(metric_mono_pair(
                move || SharedString::from(t(lang.get(), "kpi.user")),
                user,
            ))
        })
        .when_some(runtime_tone, |row, tone| {
            let rc = runtime_compat.clone();
            row.child(metric_divider()).child(metric_pair(
                move || SharedString::from(t(lang.get(), "kpi.libc")),
                ui_badge(
                    move || {
                        SharedString::from(
                            format_runtime_compat(&rc, lang.get())
                                .map(|(label, _)| label)
                                .unwrap_or_default(),
                        )
                    },
                    tone,
                ),
            ))
        })
        .when(!tags.is_empty(), |row| {
            row.child(metric_divider()).child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .children(
                        tags.into_iter()
                            .map(|tag| ui_badge(tag, BadgeTone::Neutral)),
                    )
                    .when(overflow_tags > 0, |r| {
                        r.child(ui_badge(format!("+{overflow_tags}"), BadgeTone::Neutral))
                    }),
            )
        })
}

fn metric_divider() -> impl IntoElement {
    view()
        .w(px(1.0))
        .h(px(12.0))
        .bg(move || theme().border)
        .accessibility_elements_hidden(true)
}

fn metric_pair(
    label: impl IntoReactive<SharedString>,
    value: impl IntoElement,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1p5()
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(value)
}

fn metric_mono_pair(
    label: impl IntoReactive<SharedString>,
    value: impl IntoReactive<SharedString>,
) -> impl IntoElement {
    metric_pair(
        label,
        text(value)
            .text_xs()
            .font_family(font_mono_family())
            .font_weight(FontWeight::MEDIUM)
            .tabular_nums()
            .text_color(move || theme().foreground),
    )
}

fn right_workspace(
    state: AppState,
    res: Arc<DockerAnalyzeResult>,
    summary: Arc<DockerAnalyzeSummary>,
) -> impl IntoElement {
    let active_tab = state.active_tab;

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_col()
        .bg(move || theme().background)
        .child(dynamic(move || match active_tab.get() {
            InspectorTab::FileTree => {
                file_tree::file_tree_tab(state.clone(), res.clone()).into_element()
            }
            InspectorTab::Treemap => {
                treemap::treemap_tab(state.clone(), res.clone()).into_element()
            }
            InspectorTab::Findings => {
                findings::findings_tab(state.clone(), res.clone(), summary.clone()).into_element()
            }
            InspectorTab::Dockerfile => {
                dockerfile::dockerfile_tab(state.clone(), res.clone()).into_element()
            }
        }))
}

fn bottom_status_bar(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let tab = state.active_tab;
    let source = state.current_image;
    let notice = state.status_notice;

    view()
        .h(px(24.0))
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .px_3p5()
        .bg(move || theme().background)
        .border_t_1()
        .border_color(move || theme().border)
        .child(
            text(move || match tab.get() {
                InspectorTab::FileTree => t(lang.get(), "status.hints_files"),
                _ => t(lang.get(), "status.hints_general"),
            })
            .text_size(px(11.0))
            .text_color(text_tertiary),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(when(
                    move || notice.get().is_some(),
                    move || {
                        text(move || notice.get().unwrap_or_default())
                            .text_size(px(11.0))
                            .text_color(move || theme().foreground)
                    },
                ))
                .child(ui_badge(
                    move || {
                        let kind = image_source_kind(&source.get());
                        SharedString::from(t(lang.get(), kind.i18n_key()))
                    },
                    BadgeTone::Neutral,
                )),
        )
}
