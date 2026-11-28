//! App-owned styled component library built over `nsh_base`.
//!
//! Feature modules (`features/*`) must compose UI strictly from this module
//! and `nsh_core` layout primitives (`view`, `text`, `scroll`, `virtual_list`).

pub mod button;
pub mod code;
pub mod controls;
pub mod data;
#[cfg(debug_assertions)]
pub mod gallery;
pub mod icons;
pub mod input;
pub mod motion;
pub mod overlay;
pub mod tabs;
pub mod theme;

pub use button::{
    ButtonVariant, UiSize, ui_action_card, ui_button, ui_icon_button, ui_image_pill_button,
    ui_link, ui_menu_trigger_icon, ui_recent_item, ui_sidebar_item,
};
pub use code::{ui_code_viewer, ui_diff_viewer, ui_hex_viewer};
pub use controls::{
    SegmentOption, ui_borderless_select, ui_checkbox, ui_segmented, ui_select, ui_switch,
};
pub use data::{
    BadgeTone, FileOpKind, LayerOpcodeKind, ui_badge, ui_empty_state, ui_opcode_pill, ui_panel,
    ui_panel_titled, ui_progress_bar, ui_resizable_split, ui_separator, ui_size_bar, ui_skeleton,
    ui_status_badge, ui_treemap_size_bar,
};
#[cfg(debug_assertions)]
pub use gallery::{UiGallery, ui_gallery};
pub use icons::{IconName, ui_icon, ui_spinner};
pub use input::{
    InputEscape, InputSubmit, ui_bare_input_with_ref, ui_field, ui_input, ui_input_with_ref,
    ui_search_input, ui_search_input_with_escape,
};
pub use nsh_base::{SelectItem, theme};
pub use overlay::{
    BannerTone, CommandItem, format_shortcut_hint, ui_banner, ui_command_palette, ui_context_menu,
    ui_dialog, ui_menu, ui_menu_item, ui_menu_separator, ui_popover, ui_sheet, ui_tooltip,
};
pub use tabs::{TabOption, ui_tabs};
pub use theme::{
    app_tokens, border_strong, danger, danger_soft, diverge_down, diverge_up, font_mono_family,
    info, info_soft, radius_control, radius_inset, radius_lg, register_fonts, success,
    success_soft, surface_active, surface_elevated, switch_thumb, switch_track, text_tertiary,
    treemap_ink, treemap_slot, warning, warning_soft,
};
