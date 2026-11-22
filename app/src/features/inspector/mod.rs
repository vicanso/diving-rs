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
use crate::i18n::{UiLang, localize_risk_tag, t, tf};
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, UiSize, danger, font_mono_family, success,
    text_tertiary, theme, ui_badge, ui_banner, ui_button, ui_empty_state, ui_icon,
    ui_resizable_split, warning,
};

/// Renders the full Inspector view once an analysis result is present.
pub fn inspector_workspace(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let error_state = state.clone();

    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .child(when(
            move || error_state.analyze_error.get().is_some(),
            move || {
                let dismiss = error_state.clone();
                let err_sig = error_state.analyze_error;
                view().px_4().pt_2().child(ui_banner(
                    BannerTone::Danger,
                    move || SharedString::from(err_sig.get().unwrap_or_default()),
                    Some(
                        ui_button(
                            move || SharedString::from(t(lang.get(), "common.close")),
                            ButtonVariant::Ghost,
                            UiSize::Xs,
                            Some(IconName::Close),
                            false,
                            false,
                            move || dismiss.dismiss_analyze_error(),
                        )
                        .into_element(),
                    ),
                ))
            },
        ))
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
            let left_max = px(480.0);
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
                                    move || s.retry_stale_image(),
                                )
                                .into_element(),
                            ),
                        )),
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
    let raw_tags: Vec<String> = res.tags.iter().take(3).cloned().collect();
    let overflow_tags = res.tags.len().saturating_sub(3);

    let mut items: Vec<AnyElement> = vec![
        metric_pair(
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
        )
        .into_element(),
        metric_mono_pair(
            move || SharedString::from(t(lang.get(), "kpi.image_size")),
            format_bytes(res.total_size),
        )
        .into_element(),
        metric_mono_pair(
            move || SharedString::from(t(lang.get(), "kpi.compressed")),
            format_bytes(res.size),
        )
        .into_element(),
        metric_pair(
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
        )
        .into_element(),
        metric_mono_pair(
            move || SharedString::from(t(lang.get(), "kpi.platform")),
            platform,
        )
        .into_element(),
    ];

    if !base_os.is_empty() {
        items.push(
            metric_mono_pair(
                move || SharedString::from(t(lang.get(), "kpi.base_os")),
                base_os,
            )
            .into_element(),
        );
    }
    if !user.is_empty() {
        items.push(
            metric_mono_pair(move || SharedString::from(t(lang.get(), "kpi.user")), user)
                .into_element(),
        );
    }
    if let Some(tone) = runtime_tone {
        let rc = runtime_compat.clone();
        items.push(
            metric_pair(
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
            )
            .into_element(),
        );
    }
    if !raw_tags.is_empty() {
        items.push(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .children(raw_tags.into_iter().map(move |tag| {
                    ui_badge(
                        move || SharedString::from(localize_risk_tag(lang.get(), &tag)),
                        BadgeTone::Neutral,
                    )
                }))
                .when(overflow_tags > 0, |r| {
                    r.child(ui_badge(format!("+{overflow_tags}"), BadgeTone::Neutral))
                })
                .into_element(),
        );
    }

    let item_ys: Vec<Signal<Option<f32>>> = (0..items.len()).map(|_| signal(None)).collect();

    view()
        .w_full()
        .min_h(px(36.0))
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x_3()
        .gap_y_1p5()
        .px_4()
        .py_1p5()
        .bg(move || theme().background)
        .border_b_1()
        .border_color(move || theme().border)
        .children(items.into_iter().enumerate().map(|(idx, item)| {
            let y_sig = item_ys[idx];
            let next_y_sig = item_ys.get(idx + 1).copied();
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .on_layout(move |bounds: &Bounds<Points>| {
                    let y = bounds.origin.y.0;
                    if y_sig
                        .get_untracked()
                        .is_none_or(|prev| (prev - y).abs() > 0.5)
                    {
                        y_sig.set(Some(y));
                    }
                })
                .child(item)
                .when_some(next_y_sig, |row, next_sig| {
                    // Only show the vertical divider when the following item sits on the same row.
                    row.child(
                        view()
                            .w(px(1.0))
                            .h(px(12.0))
                            .bg(move || match (y_sig.get(), next_sig.get()) {
                                (Some(y0), Some(y1)) if (y0 - y1).abs() <= 2.0 => theme().border,
                                _ => AdaptiveColor::from(transparent_black()),
                            })
                            .accessibility_elements_hidden(true),
                    )
                })
        }))
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

    view()
        .min_h(px(24.0))
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .px_3p5()
        .py_0p5()
        .bg(move || theme().background)
        .border_t_1()
        .border_color(move || theme().border)
        .child(
            text(move || match tab.get() {
                InspectorTab::FileTree => t(lang.get(), "status.hints_files"),
                _ => t(lang.get(), "status.hints_general"),
            })
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_xs()
            .text_color(text_tertiary),
        )
        .child(view().flex_shrink_0().child(ui_badge(
            move || {
                let kind = image_source_kind(&source.get());
                SharedString::from(t(lang.get(), kind.i18n_key()))
            },
            BadgeTone::Neutral,
        )))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use diving_core::{Analyzer, ImageLayer, RuntimeCompat};
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Root, Size};
    use nsh_testing::Headless;

    use super::*;
    use crate::domain::{DivingService, SharedRef};
    use crate::ui::app_tokens;

    #[component]
    fn InspectorCompactRibbonHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let res = Arc::new(DockerAnalyzeResult {
            os: "linux".to_owned(),
            arch: "amd64".to_owned(),
            base_os: "debian:12".to_owned(),
            user: "root".to_owned(),
            size: 12_000_000,
            total_size: 24_000_000,
            tags: vec![
                "[Potential Secrets]".to_owned(),
                "[Runs as Root]".to_owned(),
                "[High Layer Count: 42]".to_owned(),
            ],
            runtime_compat: RuntimeCompat {
                entrypoint: "/app/server".to_owned(),
                libc: "glibc".to_owned(),
                required_glibc: "2.38".to_owned(),
                os_glibc: "2.36".to_owned(),
                issue: "glibc-too-old".to_owned(),
                needed_libs: Vec::new(),
            },
            layers: vec![ImageLayer {
                digest: "sha256:one".to_owned(),
                cmd: "COPY . /app".to_owned(),
                size: 12_000_000,
                unpack_size: 24_000_000,
                ..Default::default()
            }],
            ..Default::default()
        });
        let summary = SharedRef::new(res.summary());
        state.result.set(Some(res));
        state.summary.set(Some(summary));
        state.current_image.set("demo:v1".to_owned());
        state.show_status_notice("Global notice only".to_owned());

        let lang_state = state.clone();
        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .child(ui_button(
                "Switch ZH",
                ButtonVariant::Ghost,
                UiSize::Sm,
                None,
                false,
                false,
                move || lang_state.lang.set(UiLang::Zh),
            ))
            .child(inspector_workspace(state))
    }

    #[test]
    fn metric_ribbon_localizes_tags_omits_duplicate_notice_and_hides_wrapped_dividers() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(960.0), Points(640.0)),
            ..EngineConfig::new(Root::of::<InspectorCompactRibbonHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start inspector ribbon harness");
        app.wait_for_within("Potential Secrets", Duration::from_secs(2))
            .expect("risk tags rendered");

        // Status notice must not be duplicated in inspector bottom_status_bar.
        assert!(!app.shows("Global notice only"));

        // Every visible 1x12 divider must have an item on the same row to its right.
        let nodes = app.screen().layout_nodes();
        let dividers: Vec<_> = nodes
            .iter()
            .filter(|n| {
                (n.bounds.size.width.0 - 1.0).abs() < 0.2
                    && (n.bounds.size.height.0 - 12.0).abs() < 0.2
            })
            .collect();
        assert!(!dividers.is_empty(), "expected metric dividers");
        for div in dividers {
            let visible = app
                .screen()
                .view_style(&format!("view:{}", div.id))
                .ok()
                .flatten()
                .is_some_and(|s| s.background_color.is_some_and(|bg| (bg & 0xff) > 0));
            if visible {
                let has_right_sibling_same_row = nodes.iter().any(|n| {
                    n.id != div.id
                        && n.text.is_some()
                        && (n.bounds.origin.y.0 - div.bounds.origin.y.0).abs() < 12.0
                        && n.bounds.origin.x.0 > div.bounds.origin.x.0
                });
                assert!(
                    has_right_sibling_same_row,
                    "visible divider at ({}, {}) has no text on its right on the same row",
                    div.bounds.origin.x.0, div.bounds.origin.y.0
                );
            }
        }

        // Risk tags localize reactively in ZH.
        app.press("Switch ZH").expect("switch to ZH");
        app.wait_for_within("疑似存在密钥", Duration::from_secs(2))
            .expect("ZH risk tag localized");
        assert!(app.shows("以 root 运行"));
        assert!(app.shows("镜像层数过多：42"));
    }
}
