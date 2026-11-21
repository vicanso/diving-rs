//! App-owned styled component library built over `nsh_base`.
//!
//! Feature modules (`features/*`) must compose UI strictly from this module
//! and `nsh_core` layout primitives (`view`, `text`, `scroll`, `virtual_list`).

pub mod button;
pub mod code;
pub mod controls;
pub mod data;
pub mod gallery;
pub mod icons;
pub mod input;
pub mod motion;
pub mod overlay;
pub mod tabs;
pub mod theme;

pub use button::{ButtonVariant, UiSize, ui_brand_home_button, ui_button, ui_icon_button, ui_link};
pub use code::{ui_code_viewer, ui_diff_viewer, ui_hex_viewer};
pub use controls::{SegmentOption, ui_checkbox, ui_segmented, ui_select, ui_switch};
pub use data::{
    BadgeTone, FileOpKind, LayerOpcodeKind, ui_badge, ui_empty_state, ui_opcode_pill, ui_panel,
    ui_panel_titled, ui_progress_bar, ui_resizable_split, ui_separator, ui_size_bar, ui_skeleton,
    ui_status_badge,
};
pub use gallery::{UiGallery, ui_gallery};
pub use icons::{IconName, ui_icon};
pub use input::{InputSubmit, ui_field, ui_input, ui_input_with_ref, ui_search_input};
pub use nsh_base::{SelectItem, theme};
pub use overlay::{
    BannerTone, CommandItem, ui_banner, ui_command_palette, ui_context_menu, ui_dialog, ui_menu,
    ui_menu_item, ui_menu_separator, ui_popover, ui_sheet, ui_tooltip,
};
pub use tabs::{TabOption, ui_tabs};
pub use theme::{
    app_tokens, border_strong, danger, danger_soft, diverge_down, diverge_up, font_mono_family,
    info, info_soft, radius_control, radius_inset, radius_lg, register_fonts, success,
    success_soft, surface_active, text_tertiary, treemap_ink, treemap_slot, warning, warning_soft,
};
