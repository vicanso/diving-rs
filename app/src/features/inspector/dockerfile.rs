//! Tab 4 (`Dockerfile`): reconstructed Dockerfile viewer with Copy, Wrap, Export, and image metadata.

use std::sync::Arc;
use std::time::Duration;

use diving_core::DockerAnalyzeResult;
use nsh_core::prelude::*;

use crate::domain::AppState;
use crate::i18n::{UiLang, t};
use crate::ui::{
    BadgeTone, ButtonVariant, IconName, UiSize, font_mono_family, radius_inset, theme, ui_badge,
    ui_button, ui_code_viewer, ui_switch,
};

/// Renders the `Dockerfile` toolbar, image metadata (`ENV` / `LABEL`), and read-only viewer.
pub fn dockerfile_tab(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let lang = state.lang;
    let wrap = state.dockerfile_wrap;
    let copied = signal(false);
    let show_meta = signal(false);

    let dockerfile_for_copy = res.dockerfile.clone();
    let dockerfile_for_export = res.dockerfile.clone();
    let dockerfile_for_view = res.dockerfile.clone();
    let user = res.user.clone();
    let envs = res.envs.clone();
    let labels = res.labels.clone();
    let meta_count = envs.len() + labels.len();
    let export_state = state;

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_col()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .py_2()
                .bg(move || theme().card)
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .when(!user.is_empty(), |r| {
                            r.child(ui_badge(
                                move || {
                                    SharedString::from(format!(
                                        "{}: {user}",
                                        t(lang.get(), "kpi.user")
                                    ))
                                },
                                BadgeTone::Neutral,
                            ))
                        })
                        .when(meta_count > 0, |r| {
                            r.child(ui_button(
                                move || {
                                    SharedString::from(format!(
                                        "{} ({meta_count})",
                                        t(lang.get(), "dockerfile.metadata")
                                    ))
                                },
                                ButtonVariant::Ghost,
                                UiSize::Xs,
                                None,
                                false,
                                false,
                                move || show_meta.set(!show_meta.get_untracked()),
                            ))
                        })
                        .child(ui_switch(
                            move || SharedString::from(t(lang.get(), "dockerfile.wrap")),
                            wrap,
                            false,
                            move |next| wrap.set(next),
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(dynamic(move || {
                            let is_copied = copied.get();
                            let payload = dockerfile_for_copy.clone();
                            ui_button(
                                move || {
                                    SharedString::from(if copied.get() {
                                        t(lang.get(), "common.copied")
                                    } else {
                                        t(lang.get(), "common.copy")
                                    })
                                },
                                ButtonVariant::Outline,
                                UiSize::Sm,
                                Some(if is_copied {
                                    IconName::Check
                                } else {
                                    IconName::Copy
                                }),
                                false,
                                false,
                                move || {
                                    set_clipboard(payload.clone());
                                    copied.set(true);
                                    set_timeout(Duration::from_millis(1500), move || {
                                        copied.set(false);
                                    });
                                },
                            )
                        }))
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "dockerfile.export")),
                            ButtonVariant::Outline,
                            UiSize::Sm,
                            Some(IconName::Download),
                            false,
                            false,
                            move || export_state.export_dockerfile(dockerfile_for_export.clone()),
                        )),
                ),
        )
        .child(when(
            move || show_meta.get() && meta_count > 0,
            move || metadata_panel(envs.clone(), labels.clone(), lang),
        ))
        .child(view().flex_1().min_h(px(0.0)).p_4().child(ui_code_viewer(
            Some(SharedString::from(dockerfile_for_view)),
            Some(SharedString::from("Dockerfile")),
            wrap,
            "Reconstructed Dockerfile viewer",
        )))
}

fn metadata_panel(
    envs: Vec<String>,
    labels: Vec<String>,
    lang: Signal<UiLang>,
) -> impl IntoElement {
    scroll()
        .max_h(px(220.0))
        .w_full()
        .bg(move || theme().background)
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .w_full()
                .px_4()
                .py_3()
                .flex_col()
                .gap_3()
                .when(!envs.is_empty(), |col| {
                    let count = envs.len();
                    col.child(
                        view()
                            .flex_col()
                            .gap_1p5()
                            .child(
                                text(move || {
                                    format!("{} ({count})", t(lang.get(), "dockerfile.envs"))
                                })
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                            )
                            .children(envs.into_iter().map(|entry| {
                                view()
                                    .px_2()
                                    .py_1()
                                    .rounded(radius_inset)
                                    .bg(move || theme().muted)
                                    .child(
                                        text(entry)
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .text_color(move || theme().foreground),
                                    )
                            })),
                    )
                })
                .when(!labels.is_empty(), |col| {
                    let count = labels.len();
                    col.child(
                        view()
                            .flex_col()
                            .gap_1p5()
                            .child(
                                text(move || {
                                    format!("{} ({count})", t(lang.get(), "dockerfile.labels"))
                                })
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                            )
                            .children(labels.into_iter().map(|entry| {
                                view()
                                    .px_2()
                                    .py_1()
                                    .rounded(radius_inset)
                                    .bg(move || theme().muted)
                                    .child(
                                        text(entry)
                                            .text_xs()
                                            .font_family(font_mono_family())
                                            .text_color(move || theme().foreground),
                                    )
                            })),
                    )
                }),
        )
}
