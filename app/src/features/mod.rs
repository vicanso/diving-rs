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
pub use sidebar::app_sidebar;
pub use title_bar::content_header_bar;

use crate::i18n::{UiLang, t};
use crate::ui::SelectItem;

/// Shared architecture selector choices used in both the Landing hero bar and compact TitleBar.
pub fn arch_select_items(lang: UiLang) -> Vec<SelectItem> {
    vec![
        SelectItem::new("", t(lang, "landing.arch_auto")),
        SelectItem::new("amd64", "linux/amd64"),
        SelectItem::new("arm64", "linux/arm64"),
        SelectItem::new("arm/v7", "linux/arm/v7"),
    ]
}
