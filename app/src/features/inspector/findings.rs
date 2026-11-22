//! Tab 3 (`Findings`): optimization recommendations, wasted space, secrets, duplicates, large files.

use std::sync::Arc;

use diving_core::{DockerAnalyzeResult, DockerAnalyzeSummary};
use nsh_core::prelude::*;

use crate::domain::{AppState, compute_what_if_projection, format_bytes};
use crate::i18n::{UiLang, localize_category, localize_severity, strip_inline_markdown, t, tf};
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, UiSize, font_mono_family, radius_inset, theme,
    ui_badge, ui_banner, ui_button, ui_empty_state, ui_panel_titled, ui_separator,
};

fn severity_badge_tone(severity: &str) -> BadgeTone {
    match severity.trim().to_ascii_lowercase().as_str() {
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
    let wide_layout = computed(move || window().width >= px(1100.0));
    let res_unstripped = res.clone();

    scroll().flex_1().min_h(px(0.0)).child(
        view()
            .w_full()
            .p_6()
            .flex_col()
            .gap_6()
            .child(recommendations_panel(res.clone(), summary.clone(), lang))
            .child(dynamic(move || {
                if wide_layout.get() {
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
            }))
            .when(!res_unstripped.unstripped_binaries.is_empty(), |col| {
                col.child(unstripped_panel(res_unstripped, lang))
            }),
    )
}

fn recommendations_panel(
    res: Arc<DockerAnalyzeResult>,
    summary: Arc<DockerAnalyzeSummary>,
    lang: Signal<UiLang>,
) -> impl IntoElement {
    let count = res.recommendations.len();
    let projection = compute_what_if_projection(&res, &summary);
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
                .when_some(projection, |col, proj| {
                    col.child(ui_banner(
                        BannerTone::Info,
                        move || {
                            SharedString::from(tf(
                                lang.get(),
                                "findings.what_if_summary",
                                &[
                                    &format_bytes(proj.saved_bytes),
                                    &format_bytes(proj.projected_size),
                                    &proj.projected_efficiency.to_string(),
                                ],
                            ))
                        },
                        None,
                    ))
                })
                .children(res.recommendations.iter().enumerate().map(|(idx, rec)| {
                    let tone = severity_badge_tone(&rec.severity);
                    let severity = rec.severity.clone();
                    let category = rec.category.clone();
                    let title = strip_inline_markdown(&rec.title);
                    let detail = strip_inline_markdown(&rec.detail);
                    let hint = rec.dockerfile_hint.clone();
                    let paths = rec.paths.clone();
                    let est_saved = rec.est_saved_bytes;
                    let is_heuristic = rec.heuristic;

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
                                        .child(ui_badge(
                                            {
                                                let sev = severity.clone();
                                                move || {
                                                    SharedString::from(localize_severity(
                                                        lang.get(),
                                                        &sev,
                                                    ))
                                                }
                                            },
                                            tone,
                                        ))
                                        .child(ui_badge(
                                            {
                                                let cat = category.clone();
                                                move || {
                                                    SharedString::from(localize_category(
                                                        lang.get(),
                                                        &cat,
                                                    ))
                                                }
                                            },
                                            BadgeTone::Neutral,
                                        ))
                                        .when(is_heuristic, |r| {
                                            r.child(ui_badge(
                                                move || {
                                                    SharedString::from(t(
                                                        lang.get(),
                                                        "findings.heuristic",
                                                    ))
                                                },
                                                BadgeTone::Neutral,
                                            ))
                                        })
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
    let show_all = signal(false);
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
            dynamic(move || {
                let limit = if show_all.get() { count } else { 40 };
                view()
                    .flex_col()
                    .gap_2()
                    .children(summary.wasted_list.iter().take(limit).enumerate().map(
                        |(idx, item)| {
                            let path = item.path.clone();
                            let size = format_bytes(item.total_size);
                            let changes = item.changes.clone();

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
                                    text(move || {
                                        let cur_lang = lang.get();
                                        let desc: Vec<String> = changes
                                            .iter()
                                            .map(|c| {
                                                let to_layer = (c.layer + 1).to_string();
                                                match (c.written_in, c.removed) {
                                                    (Some(w), true) => tf(
                                                        cur_lang,
                                                        "findings.waste_removed_from",
                                                        &[&(w + 1).to_string(), &to_layer],
                                                    ),
                                                    (Some(w), false) => tf(
                                                        cur_lang,
                                                        "findings.waste_overwritten_from",
                                                        &[&(w + 1).to_string(), &to_layer],
                                                    ),
                                                    (None, true) => tf(
                                                        cur_lang,
                                                        "findings.waste_removed_in",
                                                        &[&to_layer],
                                                    ),
                                                    (None, false) => tf(
                                                        cur_lang,
                                                        "findings.waste_overwritten_in",
                                                        &[&to_layer],
                                                    ),
                                                }
                                            })
                                            .collect();
                                        desc.join(" · ")
                                    })
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                                )
                        },
                    ))
                    .when(count > 40, |col| {
                        col.child(view().pt_2().child(ui_button(
                            move || {
                                if show_all.get() {
                                    SharedString::from(t(lang.get(), "findings.show_less"))
                                } else {
                                    SharedString::from(tf(
                                        lang.get(),
                                        "findings.show_all",
                                        &[&count.to_string()],
                                    ))
                                }
                            },
                            ButtonVariant::Ghost,
                            UiSize::Xs,
                            None,
                            false,
                            false,
                            move || show_all.set(!show_all.get_untracked()),
                        )))
                    })
                    .into_element()
            })
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
                    let reason = strip_inline_markdown(&item.reason);
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
                                    text(move || {
                                        tf(
                                            lang.get(),
                                            "findings.duplicates_copies",
                                            &[&copies, &wasted],
                                        )
                                    })
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
    let show_all = signal(false);
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
            dynamic(move || {
                let limit = if show_all.get() { count } else { 40 };
                view()
                    .flex_col()
                    .gap_2()
                    .children(
                        res.big_modified_file_list
                            .iter()
                            .take(limit)
                            .enumerate()
                            .map(|(idx, item)| {
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
                            }),
                    )
                    .when(count > 40, |col| {
                        col.child(view().pt_2().child(ui_button(
                            move || {
                                if show_all.get() {
                                    SharedString::from(t(lang.get(), "findings.show_less"))
                                } else {
                                    SharedString::from(tf(
                                        lang.get(),
                                        "findings.show_all",
                                        &[&count.to_string()],
                                    ))
                                }
                            },
                            ButtonVariant::Ghost,
                            UiSize::Xs,
                            None,
                            false,
                            false,
                            move || show_all.set(!show_all.get_untracked()),
                        )))
                    })
                    .into_element()
            })
            .into_element()
        },
    )
}

fn unstripped_panel(res: Arc<DockerAnalyzeResult>, lang: Signal<UiLang>) -> impl IntoElement {
    let count = res.unstripped_binaries.len();
    let total_debug: u64 = res.unstripped_binaries.iter().map(|b| b.debug_bytes).sum();
    ui_panel_titled(
        move || {
            SharedString::from(format!(
                "{} ({count})",
                t(lang.get(), "findings.unstripped")
            ))
        },
        Some(ui_badge(format_bytes(total_debug), BadgeTone::Info).into_element()),
        view()
            .flex_col()
            .gap_2()
            .children(
                res.unstripped_binaries
                    .iter()
                    .enumerate()
                    .map(|(idx, bin)| {
                        let path = bin.path.clone();
                        let bin_size = format_bytes(bin.size);
                        let debug_size = format_bytes(bin.debug_bytes);
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
                            .child(
                                text(move || {
                                    tf(
                                        lang.get(),
                                        "findings.unstripped_detail",
                                        &[&bin_size, &debug_size],
                                    )
                                })
                                .text_xs()
                                .font_family(font_mono_family())
                                .tabular_nums()
                                .text_color(move || theme().muted_foreground),
                            )
                    }),
            )
            .into_element(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use diving_core::{Analyzer, Recommendation, UnstrippedBinary};
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Root, Size};
    use nsh_testing::Headless;

    use super::*;
    use crate::domain::DivingService;
    use crate::ui::app_tokens;

    #[component]
    fn FindingsI18nHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        provide_context(state.clone());
        let res = Arc::new(DockerAnalyzeResult {
            recommendations: vec![Recommendation {
                id: "runasroot".to_owned(),
                category: "security".to_owned(),
                severity: "high".to_owned(),
                title: "Container *runs* as root".to_owned(),
                detail: "Add `USER nonroot` in the *same* step.".to_owned(),
                dockerfile_hint: "USER nonroot".to_owned(),
                paths: Vec::new(),
                est_saved_bytes: 0,
                heuristic: false,
            }],
            unstripped_binaries: vec![UnstrippedBinary {
                layer_index: 0,
                path: "/usr/bin/app".to_owned(),
                size: 1024,
                debug_bytes: 512,
                symbol_bytes: 0,
            }],
            ..Default::default()
        });
        let summary = Arc::new(res.summary());
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
            .child(findings_tab(state, res, summary))
    }

    #[test]
    fn findings_localizes_severity_category_unstripped_and_strips_markdown() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(Root::of::<FindingsI18nHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start findings harness");
        app.wait_for_within("HIGH", Duration::from_secs(2))
            .expect("uppercase EN severity badge");
        assert!(app.shows("Security"));
        assert!(app.shows("Container runs as root"));
        assert!(app.shows("Add USER nonroot in the same step."));
        assert!(!app.shows("`USER nonroot`"));
        assert!(!app.shows("*same*"));
        assert!(app.shows("1.0 KiB (debug 512 B)"));

        app.press("Switch ZH").expect("switch to ZH");
        app.wait_for_within("高", Duration::from_secs(2))
            .expect("ZH severity localized");
        assert!(app.shows("安全"));
        assert!(app.shows("1.0 KiB（调试信息 512 B）"));
    }
}
