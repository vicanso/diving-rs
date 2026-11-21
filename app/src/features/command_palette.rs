//! Global `Mod+K` Command Palette (`ui_command_palette`) for instant navigation and actions.

use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState, InspectorTab};
use crate::i18n::{UiLang, t};
use crate::ui::{CommandItem, ui_command_palette};

const CMD_RECENT_PREFIX: &str = "recent:";

/// Renders the reactive `Mod+K` command palette modal overlay.
pub fn command_palette_overlay(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let item_state = state.clone();
    let action_state = state.clone();

    let items = computed(move || build_command_items(&item_state));

    ui_command_palette(
        state.command_palette_open,
        items,
        move || Some(SharedString::from(t(lang.get(), "cmd.placeholder"))),
        move || SharedString::from(t(lang.get(), "cmd.empty")),
        move |id| execute_command(&action_state, id),
    )
}

fn build_command_items(state: &AppState) -> Vec<CommandItem> {
    let lang = state.lang.get();
    let cur_appearance = state.appearance.get();
    let has_result = state.result.get().is_some();
    let has_image = !state.current_image.get().is_empty();
    let recent = state.recent_images.get();
    let active_tab = state.active_tab.get();

    let group_actions = t(lang, "cmd.group_actions");
    let group_tabs = t(lang, "cmd.group_tabs");
    let group_recent = t(lang, "cmd.group_recent");
    let group_appearance = t(lang, "cmd.group_appearance");

    let mut out = vec![
        CommandItem::new("action:open_file", t(lang, "menu.open_local_file"))
            .group(group_actions)
            .keywords(["open", "tar", "oci", "file"])
            .shortcut("mod+o"),
        CommandItem::new("action:local_images", t(lang, "menu.browse_local_images"))
            .group(group_actions)
            .keywords(["docker", "podman", "local", "container"])
            .shortcut("mod+shift+o"),
        CommandItem::new("action:compare", t(lang, "menu.compare_with"))
            .group(group_actions)
            .keywords(["diff", "compare", "delta"])
            .shortcut("mod+d"),
        CommandItem::new("action:focus_search", t(lang, "cmd.focus_search"))
            .group(group_actions)
            .keywords(["search", "image", "focus"])
            .shortcut("mod+l"),
        CommandItem::new("action:reanalyze", t(lang, "cmd.reanalyze"))
            .group(group_actions)
            .keywords(["refresh", "reload", "reanalyze"])
            .shortcut("mod+r")
            .disabled(!has_image),
        CommandItem::new("action:export_md", t(lang, "cmd.export_md"))
            .group(group_actions)
            .keywords(["export", "markdown", "report"])
            .shortcut("mod+e")
            .disabled(!has_result),
        CommandItem::new("action:clear_cache", t(lang, "cmd.clear_cache"))
            .group(group_actions)
            .keywords(["clean", "cache", "gc"]),
        CommandItem::new("action:home", t(lang, "cmd.back_home"))
            .group(group_actions)
            .keywords(["home", "landing"]),
        CommandItem::new("tab:files", t(lang, "tab.files"))
            .group(group_tabs)
            .keywords(["files", "tree", "layers"])
            .shortcut("mod+1")
            .disabled(!has_result)
            .checked(has_result && active_tab == InspectorTab::FileTree),
        CommandItem::new("tab:treemap", t(lang, "tab.treemap"))
            .group(group_tabs)
            .keywords(["treemap", "space", "chart"])
            .shortcut("mod+2")
            .disabled(!has_result)
            .checked(has_result && active_tab == InspectorTab::Treemap),
        CommandItem::new("tab:findings", t(lang, "tab.findings"))
            .group(group_tabs)
            .keywords(["findings", "secrets", "wasted", "recommendations"])
            .shortcut("mod+3")
            .disabled(!has_result)
            .checked(has_result && active_tab == InspectorTab::Findings),
        CommandItem::new("tab:dockerfile", t(lang, "tab.dockerfile"))
            .group(group_tabs)
            .keywords(["dockerfile", "build"])
            .shortcut("mod+4")
            .disabled(!has_result)
            .checked(has_result && active_tab == InspectorTab::Dockerfile),
    ];

    for (idx, entry) in recent.iter().enumerate() {
        let label = if entry.arch.is_empty() {
            entry.image.clone()
        } else {
            format!("{} ({})", entry.image, entry.arch)
        };
        out.push(
            CommandItem::new(format!("{CMD_RECENT_PREFIX}{idx}"), label)
                .group(group_recent)
                .keywords([entry.image.clone(), entry.arch.clone(), "recent".to_owned()]),
        );
    }

    out.extend([
        CommandItem::new("theme:system", t(lang, "cmd.theme_system"))
            .group(group_appearance)
            .keywords(["theme", "system", "auto"])
            .checked(cur_appearance == Appearance::System),
        CommandItem::new("theme:light", t(lang, "cmd.theme_light"))
            .group(group_appearance)
            .keywords(["theme", "light"])
            .checked(cur_appearance == Appearance::Light),
        CommandItem::new("theme:dark", t(lang, "cmd.theme_dark"))
            .group(group_appearance)
            .keywords(["theme", "dark"])
            .checked(cur_appearance == Appearance::Dark),
        CommandItem::new("lang:en", t(lang, "cmd.lang_en"))
            .group(group_appearance)
            .keywords(["language", "english", "en"])
            .checked(lang == UiLang::En),
        CommandItem::new("lang:zh", t(lang, "cmd.lang_zh"))
            .group(group_appearance)
            .keywords(["language", "chinese", "zh", "中文"])
            .checked(lang == UiLang::Zh),
    ]);

    out
}

fn execute_command(state: &AppState, id: &str) {
    if let Some(raw_idx) = id.strip_prefix(CMD_RECENT_PREFIX)
        && let Ok(idx) = raw_idx.parse::<usize>()
        && let Some(entry) = state.recent_images.get_untracked().into_iter().nth(idx)
    {
        let arch = (!entry.arch.is_empty()).then_some(entry.arch);
        state.analyze(entry.image, arch);
        return;
    }

    match id {
        "action:open_file" => state.open_local_file_dialog(),
        "action:local_images" => state.open_local_picker(),
        "action:compare" => state.open_compare_dialog(),
        "action:focus_search" => state.focus_search_input(),
        "action:reanalyze" => state.reanalyze_current_image(),
        "action:export_md" => {
            if state.route.get_untracked() == AppRoute::Diff {
                state.export_diff_markdown();
            } else {
                state.export_markdown_report();
            }
        }
        "action:clear_cache" => state.clear_cache(),
        "action:home" => state.navigate_home(),
        "tab:files" => switch_tab(state, InspectorTab::FileTree),
        "tab:treemap" => switch_tab(state, InspectorTab::Treemap),
        "tab:findings" => switch_tab(state, InspectorTab::Findings),
        "tab:dockerfile" => switch_tab(state, InspectorTab::Dockerfile),
        "theme:system" => state.set_appearance(Appearance::System),
        "theme:light" => state.set_appearance(Appearance::Light),
        "theme:dark" => state.set_appearance(Appearance::Dark),
        "lang:en" => state.set_lang(UiLang::En),
        "lang:zh" => state.set_lang(UiLang::Zh),
        _ => {}
    }
}

fn switch_tab(state: &AppState, tab: InspectorTab) {
    if state.result.get_untracked().is_some() {
        state.active_tab.set(tab);
        state.route.set(AppRoute::Inspector);
    }
}
