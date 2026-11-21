//! The app's Rust UI: its root component, which `nsh run`, `nsh package`
//! and `cargo run` mount.

pub mod domain;
pub mod features;
pub mod i18n;
pub mod ui;

use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::main_window;
use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState, DivingService};
use crate::features::{
    app_sidebar, command_palette_overlay, compare_dialog, content_header_bar, diff_workspace,
    file_preview_sheet, inspector_workspace, install_desktop_menus_and_keys, landing_view,
    local_picker_dialog,
};
use crate::i18n::{t, tf};
use crate::ui::{
    BannerTone, ButtonVariant, IconName, UiGallery, UiSize, font_mono_family, register_fonts,
    theme, ui_banner, ui_button, ui_gallery, ui_progress_bar,
};

/// The app's UI set (`nsh.json` `"rust"`): what a TSX entry may import from
/// `diving/native` (`README.md`, "Adding a TSX entry").
#[native_module("diving/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
    registry.register::<UiGallery>();
}

/// The application's root component.
#[component]
pub fn App() -> impl IntoElement {
    register_fonts();

    if os().is_desktop() {
        let mut bar = TitleBarOptions::default();
        bar.style = TitleBarStyle::Hidden;
        bar.height = Some(px(48.0));

        let mut attrs = WindowAttributes::default();
        attrs.title = Some("Diving".into());
        attrs.min_size = Some(Some(size(px(960.0), px(640.0))));
        attrs.title_bar = Some(bar);

        main_window().set(attrs);
    }

    let state = AppState::new(DivingService::new());
    provide_context(state.clone());
    install_desktop_menus_and_keys(state.clone());
    let sidebar_open = state.app_sidebar_open;
    let sidebar_state = state.clone();
    let route_state = state.clone();
    let config_state = state.clone();
    let progress_state = state.clone();

    view()
        .relative()
        .flex_1()
        .flex_row()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(when(
            move || sidebar_open.get(),
            move || app_sidebar(sidebar_state.clone()),
        ))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex_col()
                .bg(move || theme().background)
                .child(content_header_bar(state.clone()))
                .child(when(
                    move || config_state.config_warning.get().is_some(),
                    move || config_warning_banner(config_state.clone()),
                ))
                .child(when(
                    move || {
                        progress_state.is_loading.get()
                            && progress_state.route.get() != AppRoute::Landing
                    },
                    move || global_progress_strip(progress_state.clone()),
                ))
                .child(
                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .flex_col()
                        .child(dynamic(move || match route_state.route.get() {
                            AppRoute::Landing => landing_view(route_state.clone()).into_element(),
                            AppRoute::Inspector => {
                                inspector_workspace(route_state.clone()).into_element()
                            }
                            AppRoute::Diff => diff_workspace(route_state.clone()).into_element(),
                            AppRoute::Gallery => ui_gallery().into_element(),
                        })),
                ),
        )
        .child(file_preview_sheet(state.clone()))
        .child(compare_dialog(state.clone()))
        .child(local_picker_dialog(state.clone()))
        .child(command_palette_overlay(state))
}

fn config_warning_banner(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let warning = state.config_warning;
    let dismiss_state = state;

    view().px_4().pt_2().child(ui_banner(
        BannerTone::Warning,
        move || {
            let err = warning.get().unwrap_or_default();
            SharedString::from(tf(lang.get(), "config.fallback_warning", &[&err]))
        },
        Some(
            ui_button(
                move || SharedString::from(t(lang.get(), "common.close")),
                ButtonVariant::Ghost,
                UiSize::Xs,
                Some(IconName::Close),
                false,
                false,
                move || dismiss_state.dismiss_config_warning(),
            )
            .into_element(),
        ),
    ))
}

fn global_progress_strip(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let pending = state.pending_image;
    let progress_text = state.progress_text;
    let cancel_state = state.clone();

    view()
        .w_full()
        .px_4()
        .py_2()
        .flex_row()
        .items_center()
        .gap_3()
        .bg(move || theme().muted)
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            text(move || {
                                let img = pending.get().unwrap_or_default();
                                if img.is_empty() {
                                    SharedString::from(t(lang.get(), "landing.analyzing"))
                                } else {
                                    SharedString::from(tf(
                                        lang.get(),
                                        "progress.analyzing_image",
                                        &[&img],
                                    ))
                                }
                            })
                            .text_xs()
                            .font_family(font_mono_family())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                        )
                        .child(
                            text(move || {
                                progress_text.get().unwrap_or_else(|| {
                                    t(lang.get(), "landing.analyzing").to_owned()
                                })
                            })
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(ui_progress_bar(
                    state.progress_ratio,
                    state.progress_indeterminate,
                    "Image analysis progress",
                )),
        )
        .child(ui_button(
            move || SharedString::from(t(lang.get(), "landing.cancel")),
            ButtonVariant::Outline,
            UiSize::Xs,
            Some(IconName::Close),
            false,
            false,
            move || cancel_state.cancel_analysis(),
        ))
}
