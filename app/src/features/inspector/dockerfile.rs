//! Tab 4 (`Dockerfile`): reconstructed Dockerfile viewer with Copy, Wrap, and Export actions.

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use diving_core::DockerAnalyzeResult;
use nsh_core::file_dialog::SaveDialog;
use nsh_core::prelude::*;

use crate::domain::AppState;
use crate::i18n::{t, tf};
use crate::ui::{
    BadgeTone, ButtonVariant, IconName, UiSize, theme, ui_badge, ui_button, ui_code_viewer,
    ui_switch,
};

/// Renders the `Dockerfile` toolbar + full-height syntax-highlighted read-only viewer.
pub fn dockerfile_tab(state: AppState, res: Arc<DockerAnalyzeResult>) -> impl IntoElement {
    let lang = state.lang;
    let wrap = state.dockerfile_wrap;
    let copied = signal(false);

    let dockerfile_for_copy = res.dockerfile.clone();
    let dockerfile_for_export = res.dockerfile.clone();
    let dockerfile_for_view = res.dockerfile.clone();
    let user = res.user.clone();
    let export_state = state.clone();

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
                            move || {
                                let body = dockerfile_for_export.clone();
                                let cur_lang = export_state.lang.get_untracked();
                                let s = export_state.clone();
                                spawn(async move {
                                    let picked = pick_save_path(
                                        SaveDialog::new()
                                            .file_name("Dockerfile")
                                            .title(t(cur_lang, "dockerfile.export")),
                                    )
                                    .await;
                                    if let Ok(Some(dest)) = picked {
                                        match fs::write(&dest, body) {
                                            Ok(()) => {
                                                let msg = tf(
                                                    cur_lang,
                                                    "notice.exported_to",
                                                    &[&dest.display().to_string()],
                                                );
                                                s.status_notice.set(Some(msg));
                                            }
                                            Err(err) => s.status_notice.set(Some(err.to_string())),
                                        }
                                    }
                                });
                            },
                        )),
                ),
        )
        .child(view().flex_1().min_h(px(0.0)).p_4().child(ui_code_viewer(
            Some(SharedString::from(dockerfile_for_view)),
            Some(SharedString::from("Dockerfile")),
            wrap,
            "Reconstructed Dockerfile viewer",
        )))
}
