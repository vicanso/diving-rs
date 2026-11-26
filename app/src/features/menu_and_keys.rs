//! Global desktop keyboard shortcuts and native localized application menu bar.

use nsh_core::desktop::MenuRole;
use nsh_core::element::desktop::{MenuItem, set_application_menu};
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState, InspectorTab, visible_layer_indices};
use crate::i18n::{UiLang, t};

/// Registers global keyboard bindings and reactive desktop menu items inside the root owner.
pub fn install_desktop_menus_and_keys(state: AppState) {
    install_application_menu(state.clone());
    bind_global_keys(state);
}

/// Registers reactive desktop menu items at the application root so the menu bar
/// survives when desktop windows close.
pub fn install_application_menu(state: AppState) {
    if os().is_desktop() {
        let menu_state = state;
        effect(move || {
            let lang = menu_state.lang.get();
            let appearance = menu_state.appearance.get();
            let has_result = menu_state.result.get().is_some();
            let can_reanalyze = menu_state.can_reanalyze();
            let on_diff = menu_state.route.get() == AppRoute::Diff;
            let can_export = menu_state.can_export_markdown();
            let active_tab = menu_state.active_tab.get();

            let items = build_app_menu(
                &menu_state,
                lang,
                appearance,
                has_result,
                can_reanalyze,
                on_diff,
                can_export,
                active_tab,
            );
            set_application_menu(items);
        });
    }
}

/// Registers per-window global keyboard bindings inside the active window's reactive tree.
pub fn bind_global_keys(state: AppState) {
    let always = KeyOptions {
        when: When::Always,
        ..Default::default()
    };

    let k_state = state.clone();
    on_key_with("mod+k", always, move |_| {
        k_state.toggle_command_palette();
    });

    let l_state = state.clone();
    on_key_with("mod+l", always, move |_| {
        l_state.focus_search_input();
    });

    let o_state = state.clone();
    on_key_with("mod+o", always, move |_| {
        o_state.open_local_file_dialog();
    });

    let shift_o_state = state.clone();
    on_key_with("mod+shift+o", always, move |_| {
        shift_o_state.open_local_picker();
    });

    let d_state = state.clone();
    on_key_with("mod+d", always, move |_| {
        d_state.open_compare_dialog();
    });

    let r_state = state.clone();
    on_key_with("mod+r", always, move |_| {
        r_state.reanalyze_current_image();
    });

    let e_state = state.clone();
    on_key_with("mod+e", always, move |_| {
        if e_state.route.get_untracked() == AppRoute::Diff {
            e_state.export_diff_markdown();
        } else {
            e_state.export_markdown_report();
        }
    });

    for (key, tab) in [
        ("mod+1", InspectorTab::FileTree),
        ("mod+2", InspectorTab::Treemap),
        ("mod+3", InspectorTab::Findings),
        ("mod+4", InspectorTab::Dockerfile),
    ] {
        let tab_state = state.clone();
        on_key_with(key, always, move |_| {
            if tab_state.result.get_untracked().is_some() {
                tab_state.active_tab.set(tab);
                tab_state.route.set(AppRoute::Inspector);
            }
        });
    }

    let slash_state = state.clone();
    on_key_with(
        "/",
        KeyOptions {
            when: When::NotEditing,
            ..Default::default()
        },
        move |_| slash_state.focus_tree_filter(),
    );

    let sidebar_state = state.clone();
    on_key_with("mod+\\", always, move |_| {
        sidebar_state.toggle_app_sidebar();
    });

    let new_state = state.clone();
    on_key_with("mod+n", always, move |_| {
        new_state.navigate_home();
    });

    let nav_options = KeyOptions {
        when: When::NotEditing,
        repeat: true,
        ..Default::default()
    };

    let up_state = state.clone();
    on_key_with("up", nav_options, move |_| {
        step_selected_layer(&up_state, -1);
    });

    let down_state = state.clone();
    on_key_with("down", nav_options, move |_| {
        step_selected_layer(&down_state, 1);
    });

    let esc_state = state.clone();
    // Keep Command Palette out of this memo so when Command Palette opens over
    // another overlay (e.g., File Preview Sheet), `effect` does not re-run and
    // `nsh_base::command`'s newer Escape binding continues to fire first.
    let esc_active = computed(move || {
        esc_state.provenance_open.get()
            || esc_state.compare_dialog_open.get()
            || esc_state.local_picker_open.get()
            || esc_state.preview_sheet_open.get()
            || esc_state.header_search_open.get()
            || (esc_state.route.get() == AppRoute::Inspector
                && esc_state.active_tab.get() == InspectorTab::FileTree
                && !esc_state.tree_keyword.get().is_empty())
    });
    let esc_handler_state = state;
    effect(move || {
        if !esc_active.get() {
            return;
        }
        let s = esc_handler_state.clone();
        on_key_with(
            "escape",
            KeyOptions {
                when: When::Always,
                ..Default::default()
            },
            move |_| {
                s.dismiss_top_overlay_or_search();
            },
        );
    });
}

fn step_selected_layer(state: &AppState, delta: isize) {
    if state.file_tree_focused.get_untracked()
        || state.any_overlay_open()
        || state.route.get_untracked() != AppRoute::Inspector
        || state.active_tab.get_untracked() != InspectorTab::FileTree
    {
        return;
    }
    let Some(res) = state.result.get_untracked() else {
        return;
    };
    let visible = visible_layer_indices(&res.layers, state.layer_filter.get_untracked());
    if visible.is_empty() {
        return;
    }
    let cur = state.selected_layer.get_untracked();
    let next = match visible.iter().position(|&idx| idx == cur) {
        Some(pos) => {
            let target_pos = (pos as isize + delta).clamp(0, (visible.len() - 1) as isize) as usize;
            visible[target_pos]
        }
        None if delta >= 0 => visible[0],
        None => *visible.last().unwrap_or(&visible[0]),
    };
    state.select_layer(next);
}

#[allow(clippy::too_many_arguments)]
fn build_app_menu(
    state: &AppState,
    lang: UiLang,
    appearance: Appearance,
    has_result: bool,
    can_reanalyze: bool,
    on_diff: bool,
    can_export: bool,
    active_tab: InspectorTab,
) -> Vec<MenuItem> {
    let open_file_s = state.clone();
    let open_folder_s = state.clone();
    let new_s = state.clone();
    let local_s = state.clone();
    let compare_s = state.clone();
    let export_s = state.clone();
    let reanalyze_s = state.clone();
    let clean_s = state.clone();
    let palette_s = state.clone();
    let sidebar_s = state.clone();
    let search_s = state.clone();
    let tab1_s = state.clone();
    let tab2_s = state.clone();
    let tab3_s = state.clone();
    let tab4_s = state.clone();
    let sys_s = state.clone();
    let light_s = state.clone();
    let dark_s = state.clone();
    let en_s = state.clone();
    let zh_s = state.clone();

    let app_menu = MenuItem::submenu(
        t(lang, "app.title"),
        [
            MenuItem::role(MenuRole::About),
            MenuItem::separator(),
            MenuItem::role(MenuRole::Hide),
            MenuItem::role(MenuRole::HideOthers),
            MenuItem::role(MenuRole::ShowAll),
            MenuItem::separator(),
            MenuItem::role(MenuRole::Quit),
        ],
    );

    let file_menu = MenuItem::submenu(
        t(lang, "menu.file"),
        [
            MenuItem::new(t(lang, "sidebar.new_analysis"))
                .shortcut("mod+n")
                .on_select(move || new_s.navigate_home()),
            MenuItem::separator(),
            MenuItem::new(t(lang, "menu.open_local_file"))
                .shortcut("mod+o")
                .on_select(move || open_file_s.open_local_file_dialog()),
            MenuItem::new(t(lang, "action.open_folder"))
                .on_select(move || open_folder_s.open_local_folder()),
            MenuItem::new(t(lang, "menu.browse_local_images"))
                .shortcut("mod+shift+o")
                .on_select(move || local_s.open_local_picker()),
            MenuItem::new(t(lang, "menu.compare_with"))
                .shortcut("mod+d")
                .on_select(move || compare_s.open_compare_dialog()),
            MenuItem::separator(),
            MenuItem::new(t(lang, "cmd.reanalyze"))
                .shortcut("mod+r")
                .disabled(!can_reanalyze)
                .on_select(move || reanalyze_s.reanalyze_current_image()),
            MenuItem::new(t(lang, "cmd.export_md"))
                .shortcut("mod+e")
                .disabled(!can_export)
                .on_select(move || {
                    if on_diff {
                        export_s.export_diff_markdown();
                    } else {
                        export_s.export_markdown_report();
                    }
                }),
            MenuItem::new(t(lang, "cmd.clear_cache")).on_select(move || clean_s.clear_cache()),
            MenuItem::separator(),
            MenuItem::role(MenuRole::Close),
        ],
    );

    let edit_menu = MenuItem::submenu(
        t(lang, "menu.edit"),
        [
            MenuItem::role(MenuRole::Undo),
            MenuItem::role(MenuRole::Redo),
            MenuItem::separator(),
            MenuItem::role(MenuRole::Cut),
            MenuItem::role(MenuRole::Copy),
            MenuItem::role(MenuRole::Paste),
            MenuItem::role(MenuRole::SelectAll),
        ],
    );

    let view_menu = MenuItem::submenu(
        t(lang, "menu.view"),
        [
            MenuItem::new(t(lang, "cmd.open_palette"))
                .shortcut("mod+k")
                .on_select(move || palette_s.toggle_command_palette()),
            MenuItem::new(t(lang, "cmd.toggle_sidebar"))
                .shortcut("mod+\\")
                .on_select(move || sidebar_s.toggle_app_sidebar()),
            MenuItem::new(t(lang, "cmd.focus_search"))
                .shortcut("mod+l")
                .on_select(move || search_s.focus_search_input()),
            MenuItem::separator(),
            MenuItem::new(t(lang, "tab.files"))
                .shortcut("mod+1")
                .disabled(!has_result)
                .checked(has_result && active_tab == InspectorTab::FileTree)
                .on_select(move || switch_tab(&tab1_s, InspectorTab::FileTree)),
            MenuItem::new(t(lang, "tab.treemap"))
                .shortcut("mod+2")
                .disabled(!has_result)
                .checked(has_result && active_tab == InspectorTab::Treemap)
                .on_select(move || switch_tab(&tab2_s, InspectorTab::Treemap)),
            MenuItem::new(t(lang, "tab.findings"))
                .shortcut("mod+3")
                .disabled(!has_result)
                .checked(has_result && active_tab == InspectorTab::Findings)
                .on_select(move || switch_tab(&tab3_s, InspectorTab::Findings)),
            MenuItem::new(t(lang, "tab.dockerfile"))
                .shortcut("mod+4")
                .disabled(!has_result)
                .checked(has_result && active_tab == InspectorTab::Dockerfile)
                .on_select(move || switch_tab(&tab4_s, InspectorTab::Dockerfile)),
            MenuItem::separator(),
            MenuItem::new(t(lang, "cmd.theme_system"))
                .checked(appearance == Appearance::System)
                .on_select(move || sys_s.set_appearance(Appearance::System)),
            MenuItem::new(t(lang, "cmd.theme_light"))
                .checked(appearance == Appearance::Light)
                .on_select(move || light_s.set_appearance(Appearance::Light)),
            MenuItem::new(t(lang, "cmd.theme_dark"))
                .checked(appearance == Appearance::Dark)
                .on_select(move || dark_s.set_appearance(Appearance::Dark)),
            MenuItem::separator(),
            MenuItem::new(t(lang, "cmd.lang_en"))
                .checked(lang == UiLang::En)
                .on_select(move || en_s.set_lang(UiLang::En)),
            MenuItem::new(t(lang, "cmd.lang_zh"))
                .checked(lang == UiLang::Zh)
                .on_select(move || zh_s.set_lang(UiLang::Zh)),
        ],
    );

    let window_menu = MenuItem::submenu(
        t(lang, "menu.window"),
        [
            MenuItem::role(MenuRole::Minimize),
            MenuItem::role(MenuRole::Zoom),
            MenuItem::separator(),
            MenuItem::role(MenuRole::ToggleFullScreen),
        ],
    );

    vec![app_menu, file_menu, edit_menu, view_menu, window_menu]
}

fn switch_tab(state: &AppState, tab: InspectorTab) {
    if state.result.get_untracked().is_some() {
        state.active_tab.set(tab);
        state.route.set(AppRoute::Inspector);
    }
}
