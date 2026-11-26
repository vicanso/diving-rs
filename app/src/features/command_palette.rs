//! Global `Mod+K` Command Palette (`ui_command_palette`) for instant navigation and actions.

use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState, InspectorTab, RecentImageEntry};
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

fn push_inspector_tabs(
    out: &mut Vec<CommandItem>,
    lang: UiLang,
    group_tabs: &'static str,
    has_result: bool,
    active_tab: InspectorTab,
) {
    out.extend([
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
    ]);
}

fn push_recent_items(
    out: &mut Vec<CommandItem>,
    group_recent: &'static str,
    recent: &[RecentImageEntry],
) {
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
}

fn build_command_items(state: &AppState) -> Vec<CommandItem> {
    let lang = state.lang.get();
    let cur_appearance = state.appearance.get();
    let has_result = state.result.get().is_some();
    let can_reanalyze = state.can_reanalyze();
    let can_export_md = state.can_export_markdown();
    let recent = state.recent_images.get();
    let active_tab = state.active_tab.get();

    let group_actions = t(lang, "cmd.group_actions");
    let group_tabs = t(lang, "cmd.group_tabs");
    let group_recent = t(lang, "cmd.group_recent");
    let group_appearance = t(lang, "cmd.group_appearance");

    let mut out = Vec::new();

    if has_result {
        push_inspector_tabs(&mut out, lang, group_tabs, has_result, active_tab);
    }

    push_recent_items(&mut out, group_recent, &recent);

    out.extend([
        CommandItem::new("action:open_file", t(lang, "menu.open_local_file"))
            .group(group_actions)
            .keywords(["open", "tar", "oci", "file"])
            .shortcut("mod+o"),
        CommandItem::new("action:open_folder", t(lang, "action.open_folder"))
            .group(group_actions)
            .keywords(["open", "folder", "directory", "oci", "layout"]),
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
            .disabled(!can_reanalyze),
        CommandItem::new("action:export_md", t(lang, "cmd.export_md"))
            .group(group_actions)
            .keywords(["export", "markdown", "report"])
            .shortcut("mod+e")
            .disabled(!can_export_md),
        CommandItem::new("action:export_json", t(lang, "menu.export_json"))
            .group(group_actions)
            .keywords(["export", "json", "report"])
            .disabled(!has_result),
        CommandItem::new("action:provenance", t(lang, "provenance.title"))
            .group(group_actions)
            .keywords(["provenance", "trace", "global", "find", "file", "layer"])
            .disabled(!has_result),
        CommandItem::new("action:toggle_sidebar", t(lang, "cmd.toggle_sidebar"))
            .group(group_actions)
            .keywords(["sidebar", "panel", "collapse", "navigation"])
            .shortcut("mod+\\"),
        CommandItem::new("action:home", t(lang, "sidebar.new_analysis"))
            .group(group_actions)
            .keywords(["home", "landing", "new"])
            .shortcut("mod+n"),
        CommandItem::new("action:clear_cache", t(lang, "cmd.clear_cache"))
            .group(group_actions)
            .keywords(["clean", "cache", "gc"]),
    ]);

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

    if !has_result {
        push_inspector_tabs(&mut out, lang, group_tabs, has_result, active_tab);
    }

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
        "action:open_folder" => state.open_local_folder(),
        "action:local_images" => state.open_local_picker(),
        "action:compare" => state.open_compare_dialog(),
        "action:focus_search" => state.focus_search_input(),
        "action:provenance" => state.open_provenance_search(),
        "action:reanalyze" => state.reanalyze_current_image(),
        "action:export_md" => {
            if state.route.get_untracked() == AppRoute::Diff {
                state.export_diff_markdown();
            } else {
                state.export_markdown_report();
            }
        }
        "action:export_json" => state.export_json_report(),
        "action:clear_cache" => state.clear_cache(),
        "action:home" => state.navigate_home(),
        "action:toggle_sidebar" => state.toggle_app_sidebar(),
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use diving_core::{DockerAnalyzeResult, ImageDiff};
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::domain::{DivingService, RecentImageEntry, SharedRef};
    use crate::ui::app_tokens;

    #[component]
    fn CommandPaletteHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.recent_images.set(vec![RecentImageEntry {
            image: "redis:alpine".to_owned(),
            arch: "amd64".to_owned(),
            score: 99,
            total_size: 1024,
            analyzed_at: "2026-11-22 12:00".to_owned(),
            stale: false,
        }]);

        // On Diff with no Inspector result, tabs move to the end and Diff actions are enabled.
        state.route.set(AppRoute::Diff);
        state.diff_base_ref.set("redis:7.2".to_owned());
        state.diff_target_ref.set("redis:7.4".to_owned());
        state
            .diff_result
            .set(Some(SharedRef::new(ImageDiff::default())));
        let diff_items = build_command_items(&state);
        assert!(diff_items[0].id.starts_with("recent:"));
        let reanalyze = diff_items
            .iter()
            .find(|i| i.id == "action:reanalyze")
            .expect("reanalyze item");
        assert!(!reanalyze.disabled);
        let export_md = diff_items
            .iter()
            .find(|i| i.id == "action:export_md")
            .expect("export_md item");
        assert!(!export_md.disabled);
        let open_folder = diff_items
            .iter()
            .find(|i| i.id == "action:open_folder")
            .expect("open_folder item");
        assert!(!open_folder.disabled);

        // With an analyzed result, Inspector Tabs lead, followed by Recent Images.
        state
            .result
            .set(Some(Arc::new(DockerAnalyzeResult::default())));
        let inspector_items = build_command_items(&state);
        assert_eq!(inspector_items[0].id.as_str(), "tab:files");
        assert!(inspector_items[4].id.starts_with("recent:"));

        state.command_palette_open.set(true);
        command_palette_overlay(state)
    }

    #[test]
    fn palette_orders_tabs_and_recents_first_and_enables_diff_actions() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<CommandPaletteHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start command palette harness");
        app.wait_for_within("File Tree", Duration::from_secs(3))
            .expect("Inspector Tabs visible at top of palette");
        assert!(app.shows("redis:alpine (amd64)"));

        // Typing filters and 2-step Esc clears query then closes palette.
        app.type_text("Command Palette", "folder")
            .expect("filter for folder command");
        app.wait_for_within("Open OCI folder…", Duration::from_secs(3))
            .expect("Open OCI folder command matched");
        app.press_key("escape").expect("1st Esc clears query");
        app.wait_for_within("File Tree", Duration::from_secs(3))
            .expect("query cleared restoring Inspector Tabs");
        app.press_key("escape").expect("2nd Esc closes palette");
        app.wait_for_gone("File Tree", Duration::from_secs(3))
            .expect("palette closed");
    }
}
