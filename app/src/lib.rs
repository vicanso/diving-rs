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
use crate::ui::{UiGallery, register_fonts, theme, ui_gallery};

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
