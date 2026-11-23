//! `Compare with…` modal dialog and `Image Diff` side-by-side comparison workspace.

use std::sync::Arc;

use diving_core::{
    BaseLayerMode, BaseLayerSkipReason, ChangeSet, DiffLayer, ImageDiff, ParityStatus,
    Recommendation,
};
use nsh_core::prelude::*;

use crate::domain::{
    AppState, DiffErrorState, DiffFileTab, SharedRef, classify_layer_opcode, format_bytes,
};
use crate::features::arch_select_items;
use crate::i18n::{
    UiLang, format_shared_base_layers, format_user_error, localize_category, localize_severity,
    strip_inline_markdown, t, tf,
};
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, SelectItem, TabOption, UiSize, diverge_down,
    diverge_up, font_mono_family, info, radius_control, radius_inset, theme, ui_badge, ui_banner,
    ui_button, ui_checkbox, ui_dialog, ui_empty_state, ui_field, ui_input, ui_opcode_pill,
    ui_panel, ui_panel_titled, ui_select, ui_separator, ui_skeleton, ui_spinner, ui_tabs,
    ui_tooltip,
};

const MINUS_SIGN: char = '−';
const DIFF_FILE_ROW_HEIGHT: f32 = 28.0;
const DIFF_FILE_MAX_HEIGHT: f32 = 280.0;

/// Formats signed byte difference using explicit `+` or typographic `−` minus sign.
pub fn format_signed_bytes(delta: i64) -> String {
    if delta == 0 {
        return "0 B".to_owned();
    }
    let mag = format_bytes(delta.unsigned_abs());
    if delta > 0 {
        format!("+{mag}")
    } else {
        format!("{MINUS_SIGN}{mag}")
    }
}

/// Formats signed integer count difference (`+3`, `−2`, `0`).
pub fn format_signed_count(delta: isize) -> String {
    if delta == 0 {
        "0".to_owned()
    } else if delta > 0 {
        format!("+{delta}")
    } else {
        format!("{MINUS_SIGN}{}", delta.unsigned_abs())
    }
}

/// Modal `Compare with…` dialog allowing arbitrary Base & Target image refs and quick multi-arch comparison.
pub fn compare_dialog(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let body_state = state.clone();
    let footer_state = state.clone();

    ui_dialog(
        state.compare_dialog_open,
        move || SharedString::from(t(lang.get(), "diff.compare_with")),
        px(560.0),
        move || compare_dialog_body(body_state.clone()).into_element(),
        move || compare_dialog_footer(footer_state.clone()).into_element(),
    )
}

fn compare_dialog_body(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let base_submit = state.clone();
    let target_submit = state.clone();
    let arch_base_state = state.clone();
    let arch_target_state = state.clone();
    let res_signal = state.result;
    let cur_image_signal = state.current_image;

    view()
        .flex_col()
        .gap_4()
        .child(ui_field(
            move || SharedString::from(t(lang.get(), "diff.base_label")),
            None,
            None::<SharedString>,
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(view().flex_1().min_w(px(0.0)).child(ui_input(
                    state.compare_base_input,
                    move || t(lang.get(), "landing.placeholder").to_owned(),
                    "Base image input",
                    UiSize::Md,
                    true,
                    Some(Box::new(move |_| base_submit.submit_compare_dialog())),
                )))
                .child(view().w(px(124.0)).child(ui_select(
                    move || arch_select_items(lang.get()),
                    state.compare_base_arch,
                    move || SharedString::from(t(lang.get(), "landing.arch_auto")),
                    "Base architecture",
                    UiSize::Md,
                    false,
                    move |next| {
                        arch_base_state
                            .compare_base_arch
                            .set(next.filter(|s| !s.is_empty()))
                    },
                ))),
        ))
        .child(ui_field(
            move || SharedString::from(t(lang.get(), "diff.target_label")),
            None,
            None::<SharedString>,
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(view().flex_1().min_w(px(0.0)).child(ui_input(
                    state.compare_target_input,
                    move || t(lang.get(), "landing.placeholder").to_owned(),
                    "Target image input",
                    UiSize::Md,
                    true,
                    Some(Box::new(move |_| target_submit.submit_compare_dialog())),
                )))
                .child(view().w(px(124.0)).child(ui_select(
                    move || arch_select_items(lang.get()),
                    state.compare_target_arch,
                    move || SharedString::from(t(lang.get(), "landing.arch_auto")),
                    "Target architecture",
                    UiSize::Md,
                    false,
                    move |next| {
                        arch_target_state
                            .compare_target_arch
                            .set(next.filter(|s| !s.is_empty()))
                    },
                ))),
        ))
        .child(dynamic(move || {
            let Some(res) = res_signal.get() else {
                return view().into_element();
            };
            if res.supported_archs.len() < 2 {
                return view().into_element();
            }
            let current_arch = res.arch.clone();
            let image_ref = cur_image_signal.get();
            if image_ref.is_empty() {
                return view().into_element();
            }
            let alternatives: Vec<String> = res
                .supported_archs
                .iter()
                .filter(|a| *a != &current_arch)
                .take(3)
                .cloned()
                .collect();
            if alternatives.is_empty() {
                return view().into_element();
            }

            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .children(alternatives.into_iter().map(|other_arch| {
                    let s = state.clone();
                    let img = image_ref.clone();
                    let cur = current_arch.clone();
                    let cur_for_click = current_arch.clone();
                    let alt = other_arch.clone();
                    ui_button(
                        move || {
                            SharedString::from(tf(lang.get(), "diff.quick_arch", &[&cur, &alt]))
                        },
                        ButtonVariant::Outline,
                        UiSize::Sm,
                        Some(IconName::Compare),
                        false,
                        false,
                        move || {
                            s.start_compare_pair(
                                img.clone(),
                                Some(other_arch.clone()),
                                img.clone(),
                                Some(cur_for_click.clone()),
                            );
                        },
                    )
                }))
                .into_element()
        }))
}

fn compare_dialog_footer(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let cancel_state = state.clone();
    let submit_state = state.clone();
    let cannot_submit = move || {
        state.compare_base_input.get().trim().is_empty()
            || state.compare_target_input.get().trim().is_empty()
    };

    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(ui_button(
            move || SharedString::from(t(lang.get(), "landing.cancel")),
            ButtonVariant::Ghost,
            UiSize::Md,
            None,
            false,
            false,
            move || cancel_state.compare_dialog_open.set(false),
        ))
        .child(ui_button(
            move || SharedString::from(t(lang.get(), "diff.compare_action")),
            ButtonVariant::Primary,
            UiSize::Md,
            Some(IconName::Compare),
            cannot_submit,
            false,
            move || submit_state.submit_compare_dialog(),
        ))
}

/// Renders the `Image Diff` workspace showing metric deltas, layer diffs, directory shifts, file changes, and recommendation deltas.
pub fn diff_workspace(state: AppState) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .child(dynamic(move || {
            let lang = state.lang;
            if state.diff_loading.get() {
                return diff_loading_view(state.clone()).into_element();
            }
            if let Some(err) = state.diff_error.get() {
                return diff_error_view(state.clone(), err).into_element();
            }
            let Some(diff) = state.diff_result.get() else {
                let open_state = state.clone();
                return view()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(ui_empty_state(
                        IconName::Compare,
                        move || SharedString::from(t(lang.get(), "diff.title")),
                        "",
                        Some(
                            ui_button(
                                move || SharedString::from(t(lang.get(), "diff.compare_with")),
                                ButtonVariant::Primary,
                                UiSize::Md,
                                Some(IconName::Compare),
                                false,
                                false,
                                move || open_state.open_compare_dialog(),
                            )
                            .into_element(),
                        ),
                    ))
                    .into_element();
            };

            render_diff_report(state.clone(), Arc::clone(&diff.0)).into_element()
        }))
}

fn diff_loading_view(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let base_ref = state.diff_base_ref;
    let target_ref = state.diff_target_ref;
    let base_progress = state.diff_base_progress;
    let target_progress = state.diff_target_progress;
    let cancel_state = state;

    view().flex_1().p_6().items_center().justify_center().child(
        view().w_full().max_w(px(620.0)).child(ui_panel_titled(
            move || SharedString::from(t(lang.get(), "diff.title")),
            Some(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        text(move || format!("{} → {}", base_ref.get(), target_ref.get()))
                            .text_xs()
                            .font_family(font_mono_family())
                            .text_color(move || theme().muted_foreground),
                    )
                    .child(ui_button(
                        move || SharedString::from(t(lang.get(), "landing.cancel")),
                        ButtonVariant::Ghost,
                        UiSize::Xs,
                        Some(IconName::Close),
                        false,
                        false,
                        move || cancel_state.cancel_diff(),
                    ))
                    .into_element(),
            ),
            view()
                .flex_col()
                .gap_3()
                .child(
                    text(move || t(lang.get(), "diff.loading"))
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                )
                .child(when(
                    move || base_progress.get().is_some(),
                    move || {
                        text(move || {
                            let msg = base_progress.get().unwrap_or_default();
                            tf(lang.get(), "diff.base_progress", &[&msg])
                        })
                        .text_xs()
                        .font_family(font_mono_family())
                        .text_color(move || theme().muted_foreground)
                    },
                ))
                .child(when(
                    move || target_progress.get().is_some(),
                    move || {
                        text(move || {
                            let msg = target_progress.get().unwrap_or_default();
                            tf(lang.get(), "diff.target_progress", &[&msg])
                        })
                        .text_xs()
                        .font_family(font_mono_family())
                        .text_color(move || theme().muted_foreground)
                    },
                ))
                .child(ui_skeleton(px(480.0), px(16.0)))
                .child(ui_skeleton(px(380.0), px(16.0)))
                .child(ui_skeleton(px(420.0), px(16.0))),
        )),
    )
}

fn diff_error_view(state: AppState, err: DiffErrorState) -> impl IntoElement {
    let lang = state.lang;
    let edit_state = state.clone();
    let retry_state = state.clone();
    let headline_key = match (&err.base, &err.target) {
        (Some(_), Some(_)) => "diff.err_both_failed",
        (Some(_), None) => "diff.err_base_failed",
        (None, Some(_)) => "diff.err_target_failed",
        (None, None) => "diff.err_both_failed",
    };

    view().flex_1().p_6().items_center().justify_center().child(
        view()
            .w_full()
            .max_w(px(640.0))
            .flex_col()
            .gap_3()
            .child(ui_banner(
                BannerTone::Danger,
                move || SharedString::from(t(lang.get(), headline_key)),
                Some(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "diff.edit_inputs")),
                            ButtonVariant::Outline,
                            UiSize::Sm,
                            None,
                            false,
                            false,
                            move || edit_state.open_compare_dialog(),
                        ))
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "landing.retry")),
                            ButtonVariant::Primary,
                            UiSize::Sm,
                            Some(IconName::Refresh),
                            false,
                            false,
                            move || retry_state.reanalyze_current_image(),
                        ))
                        .into_element(),
                ),
            ))
            .when_some(err.base, |col, msg| {
                col.child(ui_panel_titled(
                    move || SharedString::from(t(lang.get(), "diff.base_label")),
                    None,
                    text(move || format_user_error(lang.get(), &msg))
                        .text_xs()
                        .font_family(font_mono_family())
                        .text_color(move || theme().foreground),
                ))
            })
            .when_some(err.target, |col, msg| {
                col.child(ui_panel_titled(
                    move || SharedString::from(t(lang.get(), "diff.target_label")),
                    None,
                    text(move || format_user_error(lang.get(), &msg))
                        .text_xs()
                        .font_family(font_mono_family())
                        .text_color(move || theme().foreground),
                ))
            }),
    )
}

fn base_layer_mode_key(mode: BaseLayerMode) -> String {
    match mode {
        BaseLayerMode::Auto => "auto".to_owned(),
        BaseLayerMode::ForceAuto => "forceAuto".to_owned(),
        BaseLayerMode::Off => "off".to_owned(),
        BaseLayerMode::Skip(n) => format!("skip:{n}"),
    }
}

fn parse_base_layer_mode_key(key: Option<&str>) -> BaseLayerMode {
    match key.unwrap_or("auto") {
        "forceAuto" => BaseLayerMode::ForceAuto,
        "off" => BaseLayerMode::Off,
        other => other
            .strip_prefix("skip:")
            .and_then(|n| n.parse::<usize>().ok())
            .map_or(BaseLayerMode::Auto, BaseLayerMode::Skip),
    }
}

fn base_layer_mode_items(lang: UiLang) -> Vec<SelectItem> {
    let mut items = vec![
        SelectItem::new("auto", t(lang, "diff.mode_auto")),
        SelectItem::new("forceAuto", t(lang, "diff.mode_force")),
        SelectItem::new("off", t(lang, "diff.mode_off")),
    ];
    for n in 1..=5 {
        items.push(SelectItem::new(
            format!("skip:{n}"),
            tf(lang, "diff.mode_skip_n", &[&n.to_string()]),
        ));
    }
    items
}

fn skip_reason_key(reason: BaseLayerSkipReason) -> &'static str {
    match reason {
        BaseLayerSkipReason::CrossArchCreatedGap => "diff.skip_created_gap",
        BaseLayerSkipReason::CrossArchFirstLayer => "diff.skip_first_layer",
        BaseLayerSkipReason::ForcedCreatedGap => "diff.skip_forced_gap",
        BaseLayerSkipReason::ForcedFirstLayer => "diff.skip_forced_first",
        _ => "diff.skip_manual",
    }
}

fn render_diff_report(state: AppState, diff: Arc<ImageDiff>) -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .w_full()
            .p_5()
            .flex_col()
            .gap_5()
            .child(summary_card(state.clone(), Arc::clone(&diff)))
            .child(config_runtime_panel(state.clone(), Arc::clone(&diff)))
            .child(layers_diff_panel(state.clone(), Arc::clone(&diff)))
            .child(dir_changes_panel(state.clone(), Arc::clone(&diff)))
            .child(file_changes_panel(state.clone(), Arc::clone(&diff)))
            .child(recommendations_diff_panel(state, diff)),
    )
}

fn summary_card(state: AppState, diff: Arc<ImageDiff>) -> impl IntoElement {
    let lang = state.lang;
    let mode_sig = state.diff_base_layer_mode;
    let recomputing_sig = state.diff_recomputing;
    let mode_state = state.clone();

    let base_name = diff.base.name.clone();
    let base_arch = diff.base.arch.clone();
    let base_stale = diff.base.stale_as_of.clone();
    let target_name = diff.target.name.clone();
    let target_arch = diff.target.arch.clone();
    let target_stale = diff.target.stale_as_of.clone();

    let mut metrics: Vec<(&'static str, String, String, String, i64)> = vec![(
        "diff.metric_image_size",
        format_bytes(diff.base.total_size),
        format_bytes(diff.target.total_size),
        format_signed_bytes(diff.size_delta),
        diff.size_delta,
    )];
    if diff.base_layers_ignored {
        let base_upper: u64 = diff.base_layers.iter().map(|l| l.unpack_size).sum();
        let target_upper: u64 = diff.target_layers.iter().map(|l| l.unpack_size).sum();
        metrics.push((
            "diff.metric_upper_size",
            format_bytes(base_upper),
            format_bytes(target_upper),
            format_signed_bytes(diff.upper_size_delta),
            diff.upper_size_delta,
        ));
    }
    metrics.extend([
        (
            "diff.metric_compressed",
            format_bytes(diff.base.size),
            format_bytes(diff.target.size),
            format_signed_bytes(diff.target.size as i64 - diff.base.size as i64),
            diff.target.size as i64 - diff.base.size as i64,
        ),
        (
            "diff.metric_layers",
            diff.base.layer_count.to_string(),
            diff.target.layer_count.to_string(),
            format_signed_count(diff.target.layer_count as isize - diff.base.layer_count as isize),
            diff.target.layer_count as i64 - diff.base.layer_count as i64,
        ),
        (
            "diff.metric_files",
            diff.base.file_count.to_string(),
            diff.target.file_count.to_string(),
            format_signed_count(diff.target.file_count as isize - diff.base.file_count as isize),
            diff.target.file_count as i64 - diff.base.file_count as i64,
        ),
    ]);

    let parity_tone = match diff.parity.status {
        ParityStatus::Consistent => BannerTone::Success,
        ParityStatus::BinaryDiffsOnly => BannerTone::Info,
        ParityStatus::ConfigOrFileDrift => BannerTone::Warning,
    };
    let parity = diff.parity.clone();
    let content_verified = diff.content_verified;
    let content_verified_files = diff.content_verified_files;
    let content_skipped_files = diff.content_skipped_files;

    ui_panel(
        None,
        None,
        view()
            .flex_col()
            .gap_4()
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        text(move || t(lang.get(), "diff.mode_label"))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().muted_foreground),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(view().w(px(230.0)).child(ui_select(
                                move || base_layer_mode_items(lang.get()),
                                move || {
                                    Some(SharedString::from(base_layer_mode_key(mode_sig.get())))
                                },
                                "",
                                "Base layers policy",
                                UiSize::Sm,
                                move || recomputing_sig.get(),
                                move |selected| {
                                    let next = parse_base_layer_mode_key(
                                        selected.as_ref().map(|s| s.as_str()),
                                    );
                                    mode_state.set_diff_base_layer_mode(next);
                                },
                            )))
                            .child(when(
                                move || recomputing_sig.get(),
                                move || ui_spinner(px(14.0), move || theme().primary),
                            )),
                    ),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .child(diff_side_card(
                        move || SharedString::from(t(lang.get(), "diff.base_label")),
                        base_name,
                        base_arch,
                        base_stale,
                    ))
                    .child(
                        text("→")
                            .text_base()
                            .font_family(font_mono_family())
                            .text_color(move || theme().muted_foreground),
                    )
                    .child(diff_side_card(
                        move || SharedString::from(t(lang.get(), "diff.target_label")),
                        target_name,
                        target_arch,
                        target_stale,
                    )),
            )
            .child(ui_separator())
            .child(
                view()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .px_2()
                            .py_1()
                            .child(view().flex_1())
                            .child(
                                text(move || t(lang.get(), "diff.base_label"))
                                    .w(px(120.0))
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_right()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(
                                text(move || t(lang.get(), "diff.target_label"))
                                    .w(px(120.0))
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_right()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(
                                text(move || t(lang.get(), "diff.metric_delta"))
                                    .w(px(120.0))
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_right()
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .children(metrics.into_iter().map(
                        |(label_key, base_v, target_v, delta_v, raw_delta)| {
                            view()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .px_2()
                                .py_1p5()
                                .rounded(radius_inset)
                                .hover(|s| s.bg(move || theme().accent))
                                .child(
                                    text(move || t(lang.get(), label_key))
                                        .flex_1()
                                        .text_sm()
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(base_v)
                                        .w(px(120.0))
                                        .text_sm()
                                        .font_family(font_mono_family())
                                        .tabular_nums()
                                        .text_right()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(target_v)
                                        .w(px(120.0))
                                        .text_sm()
                                        .font_family(font_mono_family())
                                        .font_weight(FontWeight::MEDIUM)
                                        .tabular_nums()
                                        .text_right()
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(delta_v)
                                        .w(px(120.0))
                                        .text_sm()
                                        .font_family(font_mono_family())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .tabular_nums()
                                        .text_right()
                                        .text_color(move || delta_color(raw_delta)),
                                )
                        },
                    )),
            )
            .child(
                view()
                    .flex_col()
                    .gap_1p5()
                    .child(ui_banner(
                        parity_tone,
                        move || {
                            let l = lang.get();
                            SharedString::from(match parity.status {
                                ParityStatus::Consistent => {
                                    t(l, "diff.parity_consistent").to_owned()
                                }
                                ParityStatus::BinaryDiffsOnly => tf(
                                    l,
                                    "diff.parity_binary_only",
                                    &[&parity.expected_binary_changes.to_string()],
                                ),
                                ParityStatus::ConfigOrFileDrift => tf(
                                    l,
                                    "diff.parity_drift",
                                    &[
                                        &parity.unexpected_file_changes.to_string(),
                                        &parity.config_changes.to_string(),
                                        &parity.runtime_changes.to_string(),
                                        &parity.expected_binary_changes.to_string(),
                                    ],
                                ),
                            })
                        },
                        None,
                    ))
                    .when(
                        content_verified_files > 0 || content_skipped_files > 0,
                        |col| {
                            col.child(
                                text(move || {
                                    let l = lang.get();
                                    if content_verified {
                                        tf(
                                            l,
                                            "diff.content_verified",
                                            &[
                                                &content_verified_files.to_string(),
                                                &content_skipped_files.to_string(),
                                            ],
                                        )
                                    } else {
                                        tf(
                                            l,
                                            "diff.content_unverified",
                                            &[&content_skipped_files.to_string()],
                                        )
                                    }
                                })
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                            )
                        },
                    ),
            ),
    )
}

fn diff_side_card(
    role: impl IntoReactive<SharedString>,
    name: String,
    arch: String,
    stale: Option<String>,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(220.0))
        .p_3()
        .rounded(radius_control)
        .bg(move || theme().muted)
        .flex_col()
        .gap_1()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(role)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .when(!arch.is_empty(), |r| {
                            r.child(ui_badge(arch, BadgeTone::Neutral))
                        })
                        .when_some(stale, |r, s| r.child(ui_badge(s, BadgeTone::Warning))),
                ),
        )
        .child(
            text(name)
                .text_sm()
                .font_family(font_mono_family())
                .font_weight(FontWeight::SEMIBOLD)
                .number_of_lines(1)
                .text_color(move || theme().foreground),
        )
}

#[derive(Clone)]
enum ConfigRowSetting {
    Key(&'static str),
    Literal(String),
}

impl ConfigRowSetting {
    fn resolve(&self, lang: UiLang) -> String {
        match self {
            Self::Key(k) => t(lang, k).to_owned(),
            Self::Literal(s) => s.clone(),
        }
    }
}

#[derive(Clone)]
enum ConfigRowValue {
    Text(String),
    Bool(bool),
}

impl ConfigRowValue {
    fn resolve(&self, lang: UiLang) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Bool(true) => t(lang, "diff.healthcheck_configured").to_owned(),
            Self::Bool(false) => t(lang, "diff.healthcheck_none").to_owned(),
        }
    }

    fn tooltip_text(&self, lang: UiLang) -> Option<String> {
        let resolved = self.resolve(lang);
        if resolved == "—" || resolved.is_empty() {
            None
        } else {
            Some(resolved)
        }
    }
}

struct ConfigRuntimeRow {
    setting: ConfigRowSetting,
    base: ConfigRowValue,
    target: ConfigRowValue,
}

fn build_config_runtime_rows(diff: &ImageDiff) -> Vec<ConfigRuntimeRow> {
    let mut rows = Vec::new();
    let cd = &diff.config_diff;
    let show_opt = |s: &str| {
        if s.is_empty() {
            ConfigRowValue::Text("—".to_owned())
        } else {
            ConfigRowValue::Text(s.to_owned())
        }
    };
    let show_cmd = |v: &[String]| {
        if v.is_empty() {
            ConfigRowValue::Text("—".to_owned())
        } else {
            ConfigRowValue::Text(v.join(" "))
        }
    };
    if let Some(c) = &cd.base_os {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Key("diff.setting_base_os"),
            base: show_opt(&c.base),
            target: show_opt(&c.target),
        });
    }
    if let Some(c) = &cd.user {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Literal("USER".to_owned()),
            base: show_opt(&c.base),
            target: show_opt(&c.target),
        });
    }
    if let Some(c) = &cd.working_dir {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Literal("WORKDIR".to_owned()),
            base: show_opt(&c.base),
            target: show_opt(&c.target),
        });
    }
    if let Some(c) = &cd.entrypoint {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Literal("ENTRYPOINT".to_owned()),
            base: show_cmd(&c.base),
            target: show_cmd(&c.target),
        });
    }
    if let Some(c) = &cd.cmd {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Literal("CMD".to_owned()),
            base: show_cmd(&c.base),
            target: show_cmd(&c.target),
        });
    }
    if let Some(c) = &cd.has_healthcheck {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Literal("HEALTHCHECK".to_owned()),
            base: ConfigRowValue::Bool(c.base),
            target: ConfigRowValue::Bool(c.target),
        });
    }
    if !cd.added_ports.is_empty() || !cd.removed_ports.is_empty() {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Literal("EXPOSE".to_owned()),
            base: ConfigRowValue::Text(if cd.removed_ports.is_empty() {
                "—".to_owned()
            } else {
                cd.removed_ports.join(", ")
            }),
            target: ConfigRowValue::Text(if cd.added_ports.is_empty() {
                "—".to_owned()
            } else {
                cd.added_ports.join(", ")
            }),
        });
    }
    for env in &cd.env_changes {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Literal(format!("ENV {}", env.key)),
            base: ConfigRowValue::Text(env.base.as_deref().unwrap_or("—").to_owned()),
            target: ConfigRowValue::Text(env.target.as_deref().unwrap_or("—").to_owned()),
        });
    }
    for label in &cd.label_changes {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Literal(format!("LABEL {}", label.key)),
            base: ConfigRowValue::Text(label.base.as_deref().unwrap_or("—").to_owned()),
            target: ConfigRowValue::Text(label.target.as_deref().unwrap_or("—").to_owned()),
        });
    }
    let rd = &diff.runtime_diff;
    if let Some(c) = &rd.entrypoint {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Key("diff.setting_runtime_entrypoint"),
            base: show_opt(&c.base),
            target: show_opt(&c.target),
        });
    }
    if let Some(c) = &rd.libc {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Key("diff.setting_runtime_libc"),
            base: show_opt(&c.base),
            target: show_opt(&c.target),
        });
    }
    if let Some(c) = &rd.required_glibc {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Key("diff.setting_min_glibc"),
            base: show_opt(&c.base),
            target: show_opt(&c.target),
        });
    }
    if let Some(c) = &rd.os_glibc {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Key("diff.setting_os_glibc"),
            base: show_opt(&c.base),
            target: show_opt(&c.target),
        });
    }
    if let Some(c) = &rd.issue {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Key("diff.setting_runtime_issue"),
            base: show_opt(&c.base),
            target: show_opt(&c.target),
        });
    }
    if !rd.added_unstripped.is_empty() || !rd.removed_unstripped.is_empty() {
        rows.push(ConfigRuntimeRow {
            setting: ConfigRowSetting::Key("diff.setting_unstripped"),
            base: ConfigRowValue::Text(if rd.removed_unstripped.is_empty() {
                rd.base_unstripped_count.to_string()
            } else {
                format!(
                    "{} (−{})",
                    rd.base_unstripped_count,
                    rd.removed_unstripped.join(", ")
                )
            }),
            target: ConfigRowValue::Text(if rd.added_unstripped.is_empty() {
                rd.target_unstripped_count.to_string()
            } else {
                format!(
                    "{} (+{})",
                    rd.target_unstripped_count,
                    rd.added_unstripped.join(", ")
                )
            }),
        });
    }
    rows
}

fn config_value_cell(
    value: ConfigRowValue,
    lang: Signal<UiLang>,
    is_target: bool,
) -> impl IntoElement {
    let tip_value = value.clone();
    let has_tip = value.tooltip_text(lang.get_untracked()).is_some();

    view()
        .flex_1()
        .min_w(px(0.0))
        .when(has_tip, |el| {
            let tv = tip_value;
            el.tooltip(ui_tooltip(move || {
                SharedString::from(tv.tooltip_text(lang.get()).unwrap_or_default())
            }))
        })
        .child(
            text(move || value.resolve(lang.get()))
                .w_full()
                .text_xs()
                .font_family(font_mono_family())
                .number_of_lines(2)
                .text_color(move || {
                    if is_target {
                        theme().foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}

fn config_runtime_panel(state: AppState, diff: Arc<ImageDiff>) -> impl IntoElement {
    let lang = state.lang;
    let rows = build_config_runtime_rows(&diff);
    let is_same = diff.config_diff.same && diff.runtime_diff.same;

    ui_panel_titled(
        move || SharedString::from(t(lang.get(), "diff.config_runtime_section")),
        None,
        if is_same || rows.is_empty() {
            ui_banner(
                BannerTone::Success,
                move || SharedString::from(t(lang.get(), "diff.config_runtime_ok")),
                None,
            )
            .into_element()
        } else {
            view()
                .flex_col()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .px_2()
                        .py_1()
                        .child(
                            text(move || t(lang.get(), "diff.col_setting"))
                                .w(px(180.0))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text(move || t(lang.get(), "diff.base_label"))
                                .flex_1()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text(move || t(lang.get(), "diff.target_label"))
                                .flex_1()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .children(rows.into_iter().map(|row| {
                    let setting = row.setting;
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .px_2()
                        .py_1p5()
                        .border_b_1()
                        .border_color(move || theme().border.opacity(0.45))
                        .child(
                            text(move || setting.resolve(lang.get()))
                                .w(px(180.0))
                                .text_xs()
                                .font_family(font_mono_family())
                                .font_weight(FontWeight::MEDIUM)
                                .number_of_lines(1)
                                .text_color(move || theme().foreground),
                        )
                        .child(config_value_cell(row.base, lang, false))
                        .child(config_value_cell(row.target, lang, true))
                }))
                .into_element()
        },
    )
}

fn delta_color(delta: i64) -> AdaptiveColor {
    if delta > 0 {
        diverge_up()
    } else if delta < 0 {
        diverge_down()
    } else {
        theme().muted_foreground
    }
}

fn layers_diff_panel(state: AppState, diff: Arc<ImageDiff>) -> impl IntoElement {
    let lang = state.lang;
    let shared_count = diff.shared_layers;
    let shared_size = format_bytes(diff.shared_size);
    let base_ignored = diff.base_layers_ignored;
    let base_skip_cnt = diff.base_skipped_layers;
    let target_skip_cnt = diff.target_skipped_layers;
    let base_skip_sz = format_bytes(diff.base_skipped_size);
    let target_skip_sz = format_bytes(diff.target_skipped_size);
    let reason_key = skip_reason_key(diff.base_skip_reason);

    ui_panel_titled(
        move || SharedString::from(t(lang.get(), "diff.layers_section")),
        None,
        view()
            .flex_col()
            .gap_3()
            .child(ui_banner(
                if base_ignored || shared_count > 0 {
                    BannerTone::Info
                } else {
                    BannerTone::Warning
                },
                move || {
                    let l = lang.get();
                    SharedString::from(if base_ignored {
                        let reason = t(l, reason_key);
                        tf(
                            l,
                            "diff.skipped_base_layers",
                            &[
                                &base_skip_cnt.to_string(),
                                &base_skip_sz,
                                &target_skip_cnt.to_string(),
                                &target_skip_sz,
                                reason,
                            ],
                        )
                    } else if shared_count > 0 {
                        format_shared_base_layers(l, shared_count, &shared_size)
                    } else {
                        t(l, "diff.no_shared_layers").to_owned()
                    })
                },
                None,
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_4()
                    .child(layer_diff_column(
                        move || SharedString::from(t(lang.get(), "diff.only_in_base")),
                        &diff.base_layers,
                        BadgeTone::Info,
                    ))
                    .child(layer_diff_column(
                        move || SharedString::from(t(lang.get(), "diff.only_in_target")),
                        &diff.target_layers,
                        BadgeTone::Warning,
                    )),
            ),
    )
}

fn layer_diff_column(
    heading: impl IntoReactive<SharedString>,
    layers: &[DiffLayer],
    tone: BadgeTone,
) -> impl IntoElement {
    let count = layers.len();
    view()
        .flex_1()
        .min_w(px(280.0))
        .p_3()
        .rounded(radius_control)
        .bg(move || theme().muted)
        .flex_col()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text(heading)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(ui_badge(count.to_string(), tone)),
        )
        .children(layers.iter().map(|layer| {
            let (opcode, cleaned_cmd) = classify_layer_opcode(&layer.cmd);
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .py_1()
                .child(ui_badge(format!("L{}", layer.layer), BadgeTone::Neutral))
                .child(ui_opcode_pill(opcode))
                .child(
                    text(cleaned_cmd)
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .font_family(font_mono_family())
                        .number_of_lines(1)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format_bytes(layer.unpack_size))
                        .text_xs()
                        .font_family(font_mono_family())
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                )
        }))
}

fn dir_changes_panel(state: AppState, diff: Arc<ImageDiff>) -> impl IntoElement {
    let lang = state.lang;
    let count = diff.dirs.len();
    let max_abs_delta = diff
        .dirs
        .iter()
        .map(|d| d.delta().unsigned_abs())
        .max()
        .unwrap_or(1)
        .max(1);

    ui_panel_titled(
        move || SharedString::from(format!("{} ({count})", t(lang.get(), "diff.dirs_section"))),
        None,
        if diff.dirs.is_empty() {
            ui_empty_state(
                IconName::Folder,
                move || SharedString::from(t(lang.get(), "diff.dirs_empty")),
                "",
                None,
            )
            .into_element()
        } else {
            view()
                .flex_col()
                .gap_2()
                .children(diff.dirs.iter().map(|dir| {
                    let delta = dir.delta();
                    let ratio = ((delta.unsigned_abs() as f64) / (max_abs_delta as f64))
                        .clamp(0.04, 1.0) as f32;
                    let dir_label = if dir.path.is_empty() {
                        "/".to_owned()
                    } else {
                        format!("/{}", dir.path)
                    };
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .py_1()
                        .child(
                            text(dir_label)
                                .flex_1()
                                .min_w(px(0.0))
                                .text_xs()
                                .font_family(font_mono_family())
                                .number_of_lines(1)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(format!(
                                "{} → {}",
                                format_bytes(dir.base_size),
                                format_bytes(dir.target_size)
                            ))
                            .w(px(150.0))
                            .text_xs()
                            .font_family(font_mono_family())
                            .tabular_nums()
                            .text_right()
                            .text_color(move || theme().muted_foreground),
                        )
                        .child(diverging_bar(delta, ratio))
                        .child(
                            text(format_signed_bytes(delta))
                                .w(px(90.0))
                                .text_xs()
                                .font_family(font_mono_family())
                                .font_weight(FontWeight::MEDIUM)
                                .tabular_nums()
                                .text_right()
                                .text_color(move || delta_color(delta)),
                        )
                }))
                .into_element()
        },
    )
}

fn diverging_bar(delta: i64, ratio: f32) -> impl IntoElement {
    let half_w = 52.0f32;
    let fill_w = (half_w * ratio).max(2.0);
    view()
        .w(px(half_w * 2.0 + 1.0))
        .h(px(10.0))
        .flex_row()
        .items_center()
        .rounded(radius_inset)
        .bg(move || theme().muted)
        .overflow_hidden()
        .child(
            view()
                .w(px(half_w))
                .h_full()
                .flex_row()
                .justify_end()
                .items_center()
                .when(delta < 0, |left| {
                    left.child(view().w(px(fill_w)).h(px(6.0)).bg(diverge_down))
                }),
        )
        .child(view().w(px(1.0)).h_full().bg(move || theme().border))
        .child(
            view()
                .w(px(half_w))
                .h_full()
                .flex_row()
                .justify_start()
                .items_center()
                .when(delta > 0, |right| {
                    right.child(view().w(px(fill_w)).h(px(6.0)).bg(diverge_up))
                }),
        )
}

fn file_changes_panel(state: AppState, diff: Arc<ImageDiff>) -> impl IntoElement {
    let lang = state.lang;
    let active = state.diff_file_tab;
    let unexpected_only = state.diff_unexpected_only;
    let added_count = diff.added.count;
    let removed_count = diff.removed.count;
    let changed_count = diff.changed.count;
    let total = added_count + removed_count + changed_count;
    let has_expected_binaries = diff.parity.expected_binary_changes > 0;

    let bucket_data = computed(move || {
        let bucket: &ChangeSet = match active.get() {
            DiffFileTab::Added => &diff.added,
            DiffFileTab::Removed => &diff.removed,
            DiffFileTab::Resized => &diff.changed,
        };
        if active.get() == DiffFileTab::Resized && unexpected_only.get() {
            let files: Vec<_> = bucket
                .files
                .iter()
                .filter(|f| !f.is_expected_binary_only())
                .cloned()
                .collect();
            let shown_len = files.len();
            let total_matching = bucket
                .count
                .saturating_sub(diff.parity.expected_binary_changes)
                .max(shown_len);
            (total_matching, shown_len, SharedRef::new(files))
        } else {
            let shown_len = bucket.files.len();
            (
                bucket.count,
                shown_len,
                SharedRef::new(bucket.files.clone()),
            )
        }
    });
    let is_empty = computed(move || bucket_data.with(|(_, _, files)| files.is_empty()));

    ui_panel_titled(
        move || SharedString::from(format!("{} ({total})", t(lang.get(), "diff.files_section"))),
        Some(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_3()
                .when(has_expected_binaries, |row| {
                    row.child(when(
                        move || active.get() == DiffFileTab::Resized,
                        move || {
                            ui_checkbox(
                                move || SharedString::from(t(lang.get(), "diff.unexpected_only")),
                                move || unexpected_only.get(),
                                false,
                                false,
                                move |next| unexpected_only.set(next),
                            )
                        },
                    ))
                })
                .child(ui_tabs(
                    vec![
                        TabOption::new(
                            DiffFileTab::Added,
                            move || {
                                SharedString::from(format!(
                                    "{} ({added_count})",
                                    t(lang.get(), "diff.tab_added")
                                ))
                            },
                            None,
                        ),
                        TabOption::new(
                            DiffFileTab::Removed,
                            move || {
                                SharedString::from(format!(
                                    "{} ({removed_count})",
                                    t(lang.get(), "diff.tab_removed")
                                ))
                            },
                            None,
                        ),
                        TabOption::new(
                            DiffFileTab::Resized,
                            move || {
                                SharedString::from(format!(
                                    "{} ({changed_count})",
                                    t(lang.get(), "diff.tab_resized")
                                ))
                            },
                            None,
                        ),
                    ],
                    active,
                    "File changes category",
                    move |next| state.diff_file_tab.set(next),
                ))
                .into_element(),
        ),
        dynamic(move || {
            if is_empty.get() {
                return ui_empty_state(
                    IconName::File,
                    move || SharedString::from(t(lang.get(), "diff.files_empty")),
                    "",
                    None,
                )
                .into_element();
            }

            view()
                .flex_col()
                .gap_2()
                .child(when(
                    move || bucket_data.with(|(total_len, raw_len, _)| *total_len > *raw_len),
                    move || {
                        ui_banner(
                            BannerTone::Info,
                            move || {
                                let (total_len, raw_len) =
                                    bucket_data.with(|(t_len, r_len, _)| (*t_len, *r_len));
                                SharedString::from(tf(
                                    lang.get(),
                                    "diff.truncated_files",
                                    &[&raw_len.to_string(), &total_len.to_string()],
                                ))
                            },
                            None,
                        )
                    },
                ))
                .child(
                    view()
                        .h(move || {
                            let len = bucket_data.with(|(_, _, files)| files.len());
                            let h_pt = (len as f32 * DIFF_FILE_ROW_HEIGHT)
                                .clamp(DIFF_FILE_ROW_HEIGHT, DIFF_FILE_MAX_HEIGHT);
                            px(h_pt)
                        })
                        .child(
                            virtual_list(
                                move || bucket_data.with(|(_, _, files)| files.len()),
                                move |at| {
                                    view()
                                        .w_full()
                                        .h(px(DIFF_FILE_ROW_HEIGHT))
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .border_b_1()
                                        .border_color(move || theme().border.opacity(0.45))
                                        .child(ui_badge(
                                            move || {
                                                SharedString::from(bucket_data.with(
                                                    |(_, _, files)| {
                                                        files
                                                            .get(at.index())
                                                            .map_or_else(String::new, |row| {
                                                                format!("L{}", row.layer)
                                                            })
                                                    },
                                                ))
                                            },
                                            BadgeTone::Neutral,
                                        ))
                                        .child(
                                            text(move || {
                                                bucket_data.with(|(_, _, files)| {
                                                    files
                                                        .get(at.index())
                                                        .map_or_else(String::new, |row| {
                                                            format!("/{}", row.path)
                                                        })
                                                })
                                            })
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .number_of_lines(1)
                                            .text_color(move || theme().foreground),
                                        )
                                        .child(when(
                                            move || {
                                                active.get() == DiffFileTab::Resized
                                                    && bucket_data.with(|(_, _, files)| {
                                                        files
                                                            .get(at.index())
                                                            .is_some_and(|r| r.is_binary)
                                                    })
                                            },
                                            move || {
                                                ui_badge(
                                                    move || {
                                                        SharedString::from(t(
                                                            lang.get(),
                                                            "diff.badge_binary",
                                                        ))
                                                    },
                                                    BadgeTone::Neutral,
                                                )
                                            },
                                        ))
                                        .child(when(
                                            move || {
                                                active.get() == DiffFileTab::Resized
                                                    && bucket_data.with(|(_, _, files)| {
                                                        files.get(at.index()).is_some_and(|r| {
                                                            r.content_changed
                                                                || r.base_mode != r.target_mode
                                                                || r.base_link != r.target_link
                                                        })
                                                    })
                                            },
                                            move || {
                                                ui_badge(
                                                    move || {
                                                        let l = lang.get();
                                                        let key =
                                                            bucket_data.with(|(_, _, files)| {
                                                                match files.get(at.index()) {
                                                                    Some(r)
                                                                        if r.content_changed =>
                                                                    {
                                                                        "diff.badge_content"
                                                                    }
                                                                    Some(r)
                                                                        if r.base_mode
                                                                            != r.target_mode =>
                                                                    {
                                                                        "diff.badge_mode"
                                                                    }
                                                                    Some(r)
                                                                        if r.base_link
                                                                            != r.target_link =>
                                                                    {
                                                                        "diff.badge_link"
                                                                    }
                                                                    _ => "diff.badge_content",
                                                                }
                                                            });
                                                        SharedString::from(t(l, key))
                                                    },
                                                    BadgeTone::Warning,
                                                )
                                            },
                                        ))
                                        .child(
                                            text(move || {
                                                bucket_data.with(|(_, _, files)| {
                                                    files.get(at.index()).map_or_else(
                                                        String::new,
                                                        |row| {
                                                            format!(
                                                                "{} → {}",
                                                                format_bytes(row.base_size),
                                                                format_bytes(row.target_size)
                                                            )
                                                        },
                                                    )
                                                })
                                            })
                                            .w(px(140.0))
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .tabular_nums()
                                            .text_right()
                                            .text_color(move || theme().muted_foreground),
                                        )
                                        .child(
                                            text(move || {
                                                bucket_data.with(|(_, _, files)| {
                                                    files.get(at.index()).map_or_else(
                                                        || "0 B".to_owned(),
                                                        |row| format_signed_bytes(row.delta()),
                                                    )
                                                })
                                            })
                                            .w(px(88.0))
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .font_weight(FontWeight::MEDIUM)
                                            .tabular_nums()
                                            .text_right()
                                            .text_color(move || {
                                                let delta = bucket_data.with(|(_, _, files)| {
                                                    files.get(at.index()).map_or(0, |r| r.delta())
                                                });
                                                delta_color(delta)
                                            }),
                                        )
                                },
                            )
                            .flex_1()
                            .min_h(px(0.0))
                            .estimated_item_size(px(DIFF_FILE_ROW_HEIGHT))
                            .keys(move || {
                                bucket_data.with(|(_, _, files)| {
                                    files
                                        .iter()
                                        .map(|f| format!("{}:{}", f.layer, f.path))
                                        .collect::<Vec<String>>()
                                })
                            }),
                        ),
                )
                .into_element()
        }),
    )
}

fn recommendations_diff_panel(state: AppState, diff: Arc<ImageDiff>) -> impl IntoElement {
    let lang = state.lang;
    let total = diff.new_recommendations.len() + diff.resolved_recommendations.len();

    ui_panel_titled(
        move || SharedString::from(format!("{} ({total})", t(lang.get(), "diff.recs_section"))),
        None,
        if total == 0 {
            ui_empty_state(
                IconName::Check,
                move || SharedString::from(t(lang.get(), "diff.recs_none")),
                "",
                None,
            )
            .into_element()
        } else {
            view()
                .flex_row()
                .flex_wrap()
                .gap_4()
                .when(!diff.new_recommendations.is_empty(), |row| {
                    row.child(rec_diff_column(
                        lang,
                        "diff.recs_new",
                        &diff.new_recommendations,
                        BadgeTone::Danger,
                        false,
                    ))
                })
                .when(!diff.resolved_recommendations.is_empty(), |row| {
                    row.child(rec_diff_column(
                        lang,
                        "diff.recs_resolved",
                        &diff.resolved_recommendations,
                        BadgeTone::Success,
                        true,
                    ))
                })
                .into_element()
        },
    )
}

fn rec_diff_column(
    lang: Signal<UiLang>,
    heading_key: &'static str,
    recs: &[Recommendation],
    tone: BadgeTone,
    is_resolved: bool,
) -> impl IntoElement {
    let count = recs.len();
    view()
        .flex_1()
        .min_w(px(280.0))
        .p_3()
        .rounded(radius_control)
        .bg(move || theme().muted)
        .flex_col()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text(move || t(lang.get(), heading_key))
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(ui_badge(count.to_string(), tone)),
        )
        .children(recs.iter().map(|rec| {
            let severity = rec.severity.clone();
            let severity_for_label = severity.clone();
            let category = rec.category.clone();
            let sev_tone = if is_resolved {
                BadgeTone::Success
            } else {
                match severity.as_str() {
                    "high" => BadgeTone::Danger,
                    "medium" => BadgeTone::Warning,
                    _ => BadgeTone::Info,
                }
            };
            let title = strip_inline_markdown(&rec.title);
            let detail = strip_inline_markdown(&rec.detail);

            view()
                .flex_col()
                .gap_1()
                .py_1p5()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(ui_badge(
                            move || {
                                SharedString::from(localize_severity(
                                    lang.get(),
                                    &severity_for_label,
                                ))
                            },
                            sev_tone,
                        ))
                        .child(ui_badge(
                            move || SharedString::from(localize_category(lang.get(), &category)),
                            BadgeTone::Neutral,
                        ))
                        .child(
                            text(title)
                                .flex_1()
                                .min_w(px(120.0))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().foreground),
                        ),
                )
                .child(
                    text(detail)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .when(rec.est_saved_bytes > 0, |col| {
                    col.child(
                        text(format_bytes(rec.est_saved_bytes))
                            .text_xs()
                            .font_family(font_mono_family())
                            .text_color(info),
                    )
                })
        }))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use diving_core::{
        BaseLayerSkipReason, ChangeSet, ConfigDiff, DiffSide, FileChange, ImageDiff, ParityStatus,
        ParitySummary, Recommendation, RuntimeDiff, ScalarChange,
    };
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Root, Size};
    use nsh_testing::Headless;

    use super::*;
    use crate::domain::{AppRoute, DivingService};
    use crate::ui::app_tokens;

    #[component]
    fn DiffVirtualScrollHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.route.set(AppRoute::Diff);
        let added_files: Vec<FileChange> = (0..100)
            .map(|i| FileChange {
                path: format!("usr/share/item-{i:03}.dat"),
                base_size: 0,
                target_size: (i as u64 + 1) * 1024,
                layer: (i % 4) + 1,
                ..Default::default()
            })
            .collect();
        let diff = ImageDiff {
            base: DiffSide {
                name: "demo:v1".to_owned(),
                arch: "amd64".to_owned(),
                total_size: 10_000,
                size: 5_000,
                layer_count: 1,
                file_count: 1,
                ..Default::default()
            },
            target: DiffSide {
                name: "demo:v2".to_owned(),
                arch: "amd64".to_owned(),
                total_size: 20_000,
                size: 9_000,
                layer_count: 2,
                file_count: 101,
                ..Default::default()
            },
            size_delta: 10_000,
            shared_layers: 1,
            shared_size: 4096,
            added: ChangeSet {
                count: 100,
                bytes: 500_000,
                files: added_files,
            },
            removed: ChangeSet {
                count: 1,
                bytes: -1024,
                files: vec![FileChange {
                    path: "tmp/only-one.txt".to_owned(),
                    base_size: 1024,
                    target_size: 0,
                    layer: 1,
                    ..Default::default()
                }],
            },
            resolved_recommendations: vec![Recommendation {
                id: "runasroot".to_owned(),
                severity: "high".to_owned(),
                category: "security".to_owned(),
                title: "Use `USER nonroot`".to_owned(),
                detail: "Run as *nonroot* user.".to_owned(),
                est_saved_bytes: 0,
                ..Default::default()
            }],
            ..Default::default()
        };
        state.diff_result.set(Some(SharedRef::new(diff)));
        diff_workspace(state)
    }

    #[component]
    fn CrossArchParityHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.route.set(AppRoute::Diff);
        let diff = ImageDiff {
            base: DiffSide {
                name: "svc:amd64".to_owned(),
                arch: "amd64".to_owned(),
                total_size: 12_000,
                size: 6_000,
                layer_count: 2,
                file_count: 2,
                ..Default::default()
            },
            target: DiffSide {
                name: "svc:arm64".to_owned(),
                arch: "arm64".to_owned(),
                total_size: 20_000,
                size: 9_000,
                layer_count: 2,
                file_count: 2,
                ..Default::default()
            },
            size_delta: 8_000,
            upper_size_delta: 400,
            base_layers_ignored: true,
            base_skip_reason: BaseLayerSkipReason::CrossArchCreatedGap,
            base_skipped_layers: 1,
            target_skipped_layers: 1,
            base_skipped_size: 8_000,
            target_skipped_size: 15_600,
            base_layers: vec![DiffLayer {
                layer: 2,
                digest: "sha256:app-amd".to_owned(),
                cmd: "COPY server /usr/bin/server".to_owned(),
                size: 500,
                unpack_size: 4_000,
            }],
            target_layers: vec![DiffLayer {
                layer: 2,
                digest: "sha256:app-arm".to_owned(),
                cmd: "COPY server /usr/bin/server".to_owned(),
                size: 600,
                unpack_size: 4_400,
            }],
            changed: ChangeSet {
                count: 2,
                bytes: 400,
                files: vec![
                    FileChange {
                        path: "usr/bin/server".to_owned(),
                        base_size: 1_000,
                        target_size: 1_400,
                        layer: 2,
                        base_mode: "-rwxr-xr-x".to_owned(),
                        target_mode: "-rwxr-xr-x".to_owned(),
                        is_binary: true,
                        ..Default::default()
                    },
                    FileChange {
                        path: "app/settings.json".to_owned(),
                        base_size: 32,
                        target_size: 32,
                        layer: 2,
                        base_mode: "-rw-r--r--".to_owned(),
                        target_mode: "-rw-r--r--".to_owned(),
                        content_changed: true,
                        ..Default::default()
                    },
                ],
            },
            config_diff: ConfigDiff {
                same: false,
                user: Some(ScalarChange {
                    base: "app".to_owned(),
                    target: "root".to_owned(),
                }),
                ..Default::default()
            },
            parity: ParitySummary {
                status: ParityStatus::ConfigOrFileDrift,
                expected_binary_changes: 1,
                unexpected_file_changes: 1,
                same_size_content_changes: 1,
                config_changes: 1,
                ..Default::default()
            },
            content_verified: true,
            content_verified_files: 1,
            ..Default::default()
        };
        state.diff_result.set(Some(SharedRef::new(diff)));
        diff_workspace(state)
    }

    #[test]
    fn diff_virtual_list_recycles_cleanly_across_100_rows_and_localizes_recs() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(Root::of::<DiffVirtualScrollHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start diff virtual scroll harness");
        app.wait_for_within("1 shared base layer (4.0 KiB)", Duration::from_secs(3))
            .expect("singular shared layer grammar rendered");
        assert!(app.shows("Use USER nonroot"));
        assert!(app.shows("Run as nonroot user."));
        assert!(app.shows("Security"));
        assert!(app.shows("/usr/share/item-000.dat"));

        // Scroll File Changes virtual_list past row 80 to verify cell recycling renders updated rows.
        app.scroll("/usr/share/item-000.dat", px(2400.0))
            .expect("scroll File Changes virtual list");
        app.wait_for_within("/usr/share/item-095.dat", Duration::from_secs(3))
            .expect("recycled virtual_list renders bottom rows");

        // Switch to 1-row Removed tab without tearing down virtual_list
        app.press("Removed (1)").expect("switch to Removed tab");
        app.wait_for_within("/tmp/only-one.txt", Duration::from_secs(3))
            .expect("removed row renders accurately after tab switch");

        app.press("Added (100)").expect("switch back to Added tab");
        app.wait_for_within("/usr/share/item-000.dat", Duration::from_secs(3))
            .expect("added row 000 restored");
    }

    #[test]
    fn cross_arch_diff_renders_parity_banner_config_diff_and_unexpected_filter() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(900.0)),
            ..EngineConfig::new(Root::of::<CrossArchParityHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start cross arch parity harness");
        app.wait_for_within(
            "text:/^Parity: Config or file drift/",
            Duration::from_secs(3),
        )
        .expect("render parity banner");
        assert!(app.shows("Upper Layers Size"));
        assert!(app.shows("Config & Runtime Parity"));
        assert!(app.shows("USER"));
        assert!(app.shows(
            "Skipped leading base layers: 1 base (7.8 KiB) / 1 target (15.2 KiB) · detected >1h layer build gap"
        ));

        app.press("Modified (2)").expect("switch to Modified tab");
        app.wait_for_within("/usr/bin/server", Duration::from_secs(3))
            .expect("render expected binary row");
        assert!(app.shows("/app/settings.json"));

        app.press("Unexpected only")
            .expect("toggle Unexpected only filter");
        app.wait_for_gone("/usr/bin/server", Duration::from_secs(3))
            .expect("filter out expected binary row");
        assert!(app.shows("/app/settings.json"));
    }

    #[component]
    fn ConfigLocalizationAndTruncationHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.route.set(AppRoute::Diff);
        let mut changed_files: Vec<FileChange> = (0..48)
            .map(|i| FileChange {
                path: format!("usr/bin/tool-{i:02}"),
                base_size: 10_000,
                target_size: 10_500 + i as u64,
                layer: 2,
                base_mode: "-rwxr-xr-x".to_owned(),
                target_mode: "-rwxr-xr-x".to_owned(),
                is_binary: true,
                ..Default::default()
            })
            .collect();
        changed_files.push(FileChange {
            path: "etc/app.conf".to_owned(),
            base_size: 64,
            target_size: 64,
            layer: 2,
            base_mode: "-rw-r--r--".to_owned(),
            target_mode: "-rw-r--r--".to_owned(),
            content_changed: true,
            ..Default::default()
        });
        changed_files.push(FileChange {
            path: "etc/other.conf".to_owned(),
            base_size: 128,
            target_size: 128,
            layer: 2,
            base_mode: "-rw-r--r--".to_owned(),
            target_mode: "-rw-r--r--".to_owned(),
            content_changed: true,
            ..Default::default()
        });

        let diff = ImageDiff {
            base: DiffSide {
                name: "svc:v1".to_owned(),
                arch: "amd64".to_owned(),
                ..Default::default()
            },
            target: DiffSide {
                name: "svc:v2".to_owned(),
                arch: "arm64".to_owned(),
                ..Default::default()
            },
            config_diff: ConfigDiff {
                same: false,
                base_os: Some(ScalarChange {
                    base: "debian 12".to_owned(),
                    target: "alpine 3.20".to_owned(),
                }),
                has_healthcheck: Some(ScalarChange {
                    base: false,
                    target: true,
                }),
                ..Default::default()
            },
            runtime_diff: RuntimeDiff {
                same: false,
                entrypoint: Some(ScalarChange {
                    base: "/usr/bin/app".to_owned(),
                    target: "/usr/local/bin/app".to_owned(),
                }),
                libc: Some(ScalarChange {
                    base: "glibc".to_owned(),
                    target: "musl".to_owned(),
                }),
                required_glibc: Some(ScalarChange {
                    base: "2.34".to_owned(),
                    target: "—".to_owned(),
                }),
                os_glibc: Some(ScalarChange {
                    base: "2.36".to_owned(),
                    target: "—".to_owned(),
                }),
                issue: Some(ScalarChange {
                    base: "none".to_owned(),
                    target: "glibc binary on musl".to_owned(),
                }),
                base_unstripped_count: 1,
                target_unstripped_count: 2,
                added_unstripped: vec!["/usr/local/bin/app".to_owned()],
                ..Default::default()
            },
            changed: ChangeSet {
                count: 80,
                bytes: 25_000,
                files: changed_files,
            },
            parity: ParitySummary {
                status: ParityStatus::ConfigOrFileDrift,
                expected_binary_changes: 78,
                unexpected_file_changes: 2,
                same_size_content_changes: 2,
                config_changes: 2,
                runtime_changes: 6,
                ..Default::default()
            },
            ..Default::default()
        };
        state.diff_result.set(Some(SharedRef::new(diff)));
        let lang_state = state.clone();

        view()
            .flex_col()
            .flex_1()
            .child(ui_button(
                "ToggleZH",
                ButtonVariant::Ghost,
                UiSize::Xs,
                None,
                false,
                false,
                move || lang_state.set_lang(UiLang::Zh),
            ))
            .child(diff_workspace(state))
    }

    #[test]
    fn config_runtime_table_localizes_and_unexpected_filter_updates_truncation_banner() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(900.0)),
            ..EngineConfig::new(
                Root::of::<ConfigLocalizationAndTruncationHarness>().tokens(app_tokens()),
            )
        };
        let mut app = Headless::start(config).expect("start config/truncation harness");
        app.wait_for_within("Base OS", Duration::from_secs(3))
            .expect("EN Base OS row visible");
        assert!(app.shows("Runtime entrypoint"));
        assert!(app.shows("Runtime libc"));
        assert!(app.shows("Min glibc"));
        assert!(app.shows("OS glibc"));
        assert!(app.shows("Runtime issue"));
        assert!(app.shows("Unstripped binaries"));
        assert!(app.shows("Configured"));
        assert!(app.shows("Not configured"));

        // Switch to ZH and confirm all Config & Runtime labels and HEALTHCHECK booleans localize.
        app.press("ToggleZH").expect("switch to ZH");
        app.wait_for_within("基础系统", Duration::from_secs(3))
            .expect("ZH Base OS label");
        assert!(app.shows("运行时入口"));
        assert!(app.shows("运行时 libc"));
        assert!(app.shows("最低 glibc"));
        assert!(app.shows("系统 glibc"));
        assert!(app.shows("运行时问题"));
        assert!(app.shows("未剥离符号的二进制"));
        assert!(app.shows("已配置"));
        assert!(app.shows("未配置"));

        // On Modified tab, pre-filter shows 50 of 80; toggling Unexpected only hides the stale banner.
        app.press("文件变更 (80)").expect("switch to Modified tab");
        app.wait_for_within(
            "当前仅显示变化最显著的 50 / 80 个文件。",
            Duration::from_secs(3),
        )
        .expect("pre-filter truncation banner shown");
        app.press("仅看非预期差异").expect("toggle Unexpected only");
        app.wait_for_gone(
            "当前仅显示变化最显著的 50 / 80 个文件。",
            Duration::from_secs(3),
        )
        .expect("stale pre-filter truncation banner hidden when all 2 unexpected files fit");
        assert!(app.shows("/etc/app.conf"));
        assert!(app.shows("/etc/other.conf"));
    }
}
