//! Desktop application screens and overlays built purely over `crate::ui::*`,
//! `crate::domain::*`, `crate::i18n::*`, and `nsh_core` layout primitives.

pub mod command_palette;
pub mod diff;
pub mod file_preview_sheet;
pub mod inspector;
pub mod landing;
pub mod local_picker;
pub mod menu_and_keys;
pub mod provenance_dialog;
pub mod registry_auth_dialog;
pub mod sidebar;
pub mod title_bar;

pub use command_palette::command_palette_overlay;
pub use diff::{compare_dialog, diff_workspace};
pub use file_preview_sheet::file_preview_sheet;
pub use inspector::inspector_workspace;
pub use landing::landing_view;
pub use local_picker::local_picker_dialog;
pub use menu_and_keys::{
    bind_global_keys, install_application_menu, install_desktop_menus_and_keys,
};
pub use provenance_dialog::provenance_dialog;
pub use registry_auth_dialog::registry_auth_dialog;
pub use sidebar::app_sidebar;
pub use title_bar::content_header_bar;

use std::rc::Rc;

use nsh_core::prelude::*;

use crate::domain::{
    AppState, RegistryRetryTarget, extract_registry_host_from_image, is_registry_auth_or_tls_error,
};
use crate::i18n::{UiLang, t};
use crate::ui::{ButtonVariant, IconName, SelectItem, UiSize, ui_button};

/// Resolves the image reference associated with the active analyze error.
pub fn active_failed_analyze_image(
    failed_image: Option<String>,
    current_image: String,
    image_input: String,
) -> String {
    failed_image
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!current_image.trim().is_empty()).then_some(current_image))
        .unwrap_or(image_input)
}

/// Renders a `"Configure Auth…"` button when `err_text` looks like an auth/TLS error on a registry host.
pub fn registry_auth_fix_button(
    state: AppState,
    retry_target: RegistryRetryTarget,
    image_ref: impl Fn() -> String + 'static,
    err_text: impl Fn() -> String + 'static,
) -> impl IntoElement {
    let lang = state.lang;
    let action_state = state;
    let image_ref = Rc::new(image_ref);
    let image_ref_btn = Rc::clone(&image_ref);

    when(
        move || {
            let err = err_text();
            let img = (*image_ref)();
            is_registry_auth_or_tls_error(&err) && extract_registry_host_from_image(&img).is_some()
        },
        move || {
            let s = action_state.clone();
            let img_for_click = (*image_ref_btn)();
            ui_button(
                move || SharedString::from(t(lang.get(), "error.configure_auth")),
                ButtonVariant::Outline,
                UiSize::Xs,
                Some(IconName::FileKey),
                false,
                false,
                move || {
                    if let Some(host) = extract_registry_host_from_image(&img_for_click) {
                        s.open_registry_settings_for_host(&host, retry_target);
                    }
                },
            )
        },
    )
}

/// Shared architecture selector choices used in both the Landing hero bar and compact TitleBar.
pub fn arch_select_items(lang: UiLang) -> Vec<SelectItem> {
    vec![
        SelectItem::new("", t(lang, "landing.arch_auto")),
        SelectItem::new("amd64", "linux/amd64"),
        SelectItem::new("arm64", "linux/arm64"),
        SelectItem::new("arm/v7", "linux/arm/v7"),
    ]
}
