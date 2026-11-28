//! Persistent collapsible full-height left App Sidebar (OrbStack / Linear workbench shell).

use nsh_core::element::desktop::title_bar;
use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState, RecentImageEntry, UpdatePhase, format_bytes};
use crate::features::title_bar::more_actions_menu;
use crate::i18n::{UiLang, app_version_badge, t, tf};
use crate::ui::{
    ButtonVariant, IconName, SegmentOption, UiSize, font_mono_family, radius_inset, text_tertiary,
    theme, ui_button, ui_context_menu, ui_icon, ui_icon_button, ui_menu_item, ui_progress_bar,
    ui_recent_item, ui_segmented, ui_sidebar_item, ui_spinner,
};

fn appearance_icon(appearance: Appearance) -> IconName {
    match appearance {
        Appearance::System => IconName::Monitor,
        Appearance::Light => IconName::Sun,
        Appearance::Dark => IconName::Moon,
    }
}

/// Default width of the collapsible left App Sidebar.
pub const APP_SIDEBAR_WIDTH: f32 = 208.0;

/// Renders the full-height collapsible left desktop sidebar under macOS traffic lights.
pub fn app_sidebar(state: AppState) -> impl IntoElement {
    view()
        .w(px(APP_SIDEBAR_WIDTH))
        .h_full()
        .flex_shrink_0()
        .flex_col()
        .bg(move || theme().sidebar)
        .border_r_1()
        .border_color(move || theme().sidebar_border)
        .child(sidebar_top_drag_bar(state.clone()))
        .child(sidebar_workbench_nav(state.clone()))
        .child(sidebar_recent_list(state.clone()))
        .child(sidebar_footer(state))
}

fn sidebar_top_drag_bar(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let toggle_state = state.clone();
    let new_state = state;

    view()
        .h(px(48.0))
        .w_full()
        .flex_row()
        .items_center()
        .justify_end()
        .gap_1()
        .pl(move || px(title_bar().left.0.max(12.0)))
        .pr_2p5()
        .window_drag(true)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .window_drag(false)
                .child(ui_icon_button(
                    IconName::PanelLeft,
                    "Toggle sidebar",
                    Some(move || SharedString::from(t(lang.get(), "sidebar.toggle_tooltip"))),
                    ButtonVariant::Ghost,
                    UiSize::Sm,
                    false,
                    false,
                    move || toggle_state.toggle_app_sidebar(),
                ))
                .child(ui_icon_button(
                    IconName::Plus,
                    "New analysis",
                    Some(move || SharedString::from(t(lang.get(), "sidebar.new_analysis"))),
                    ButtonVariant::Ghost,
                    UiSize::Sm,
                    false,
                    false,
                    move || new_state.navigate_home(),
                )),
        )
}

fn section_heading(label: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(label)
        .px_2()
        .pb_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .letter_spacing(px(0.3))
        .text_color(move || theme().muted_foreground)
}

fn sidebar_workbench_nav(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let route = state.route;
    let new_state = state.clone();
    let local_state = state.clone();
    let cmp_state = state.clone();
    let file_state = state.clone();
    let folder_state = state.clone();
    let reg_state = state.clone();

    view()
        .px_2p5()
        .pt_1()
        .pb_2()
        .flex_col()
        .gap_0p5()
        .child(section_heading(move || {
            SharedString::from(t(lang.get(), "sidebar.section_workbench"))
        }))
        .child(ui_sidebar_item(
            move || SharedString::from(t(lang.get(), "sidebar.new_analysis")),
            IconName::Search,
            Some("mod+n"),
            move || route.get() == AppRoute::Landing,
            move || new_state.navigate_home(),
        ))
        .child(ui_sidebar_item(
            move || SharedString::from(t(lang.get(), "sidebar.local_images")),
            IconName::Container,
            Some("mod+shift+o"),
            state.local_picker_open,
            move || local_state.open_local_picker(),
        ))
        .child(ui_sidebar_item(
            move || SharedString::from(t(lang.get(), "sidebar.compare")),
            IconName::Compare,
            Some("mod+d"),
            move || route.get() == AppRoute::Diff,
            move || {
                if cmp_state.diff_result.get_untracked().is_some() {
                    cmp_state.route.set(AppRoute::Diff);
                } else {
                    cmp_state.open_compare_dialog();
                }
            },
        ))
        .child(ui_sidebar_item(
            move || SharedString::from(t(lang.get(), "sidebar.open_file")),
            IconName::FolderInput,
            Some("mod+o"),
            false,
            move || file_state.open_local_file_dialog(),
        ))
        .child(ui_sidebar_item(
            move || SharedString::from(t(lang.get(), "action.open_folder")),
            IconName::FolderOpen,
            None,
            false,
            move || folder_state.open_local_folder(),
        ))
        .child(ui_sidebar_item(
            move || SharedString::from(t(lang.get(), "menu.registries")),
            IconName::FileKey,
            Some("mod+,"),
            state.registry_dialog_open,
            move || reg_state.open_registry_settings(),
        ))
}

fn format_recent_meta(entry: &RecentImageEntry, lang: UiLang) -> String {
    let stale_suffix = if entry.stale {
        format!(" · {}", t(lang, "recent.stale_badge"))
    } else {
        String::new()
    };
    if entry.arch.is_empty() {
        format!("{}{stale_suffix}", format_bytes(entry.total_size))
    } else {
        format!(
            "{} · {}{stale_suffix}",
            entry.arch,
            format_bytes(entry.total_size)
        )
    }
}

fn sidebar_recent_list(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let recent = state.recent_images;
    let route = state.route;
    let current_image = state.current_image;
    let arch_input = state.arch_input;
    let result_sig = state.result;
    let clear_state = state.clone();

    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .pt_2()
        .child(
            view()
                .px_2p5()
                .flex_row()
                .items_center()
                .justify_between()
                .child(section_heading(move || {
                    SharedString::from(t(lang.get(), "sidebar.section_recent"))
                }))
                .child(when(
                    move || !recent.get().is_empty(),
                    move || {
                        let s = clear_state.clone();
                        view()
                            .opacity(0.72)
                            .hover(|st| st.opacity(1.0))
                            .child(ui_button(
                                move || SharedString::from(t(lang.get(), "recent.clear_all")),
                                ButtonVariant::Ghost,
                                UiSize::Xs,
                                None,
                                false,
                                false,
                                move || s.clear_recent_images(),
                            ))
                    },
                )),
        )
        .child(
            scroll().flex_1().min_h(px(0.0)).child(
                view()
                    .px_2p5()
                    .pb_2()
                    .flex_col()
                    .gap_0p5()
                    .child(when(
                        move || recent.get().is_empty(),
                        move || {
                            text(move || t(lang.get(), "sidebar.recent_empty"))
                                .px_2()
                                .py_1p5()
                                .text_xs()
                                .text_color(text_tertiary)
                        },
                    ))
                    .child(each(
                        move || recent.get(),
                        |entry: &RecentImageEntry| format!("{}@{}", entry.image, entry.arch),
                        move |entry, _| {
                            let row_state = state.clone();
                            dynamic(move || {
                                let snap = entry.get();
                                let ui_lang = lang.get();
                                let click_state = row_state.clone();
                                let remove_state = row_state.clone();
                                let click_entry = snap.clone();
                                let remove_img = snap.image.clone();
                                let remove_arch = snap.arch.clone();
                                let entry_name = snap.image.clone();
                                let entry_arch = snap.arch.clone();
                                let meta = format_recent_meta(&snap, ui_lang);
                                let score_label = format!("{}%", snap.score);

                                ui_context_menu(
                                    ui_recent_item(
                                        SharedString::from(snap.image),
                                        SharedString::from(meta),
                                        SharedString::from(score_label),
                                        snap.score,
                                        move || {
                                            let active_arch = arch_input
                                                .get()
                                                .map(|a| a.as_str().trim().to_owned())
                                                .filter(|a| !a.is_empty())
                                                .or_else(|| {
                                                    result_sig.get().map(|r| r.arch.clone())
                                                })
                                                .unwrap_or_default();
                                            route.get() == AppRoute::Inspector
                                                && current_image.get() == entry_name
                                                && active_arch == entry_arch
                                        },
                                        move || {
                                            let arch = (!click_entry.arch.is_empty())
                                                .then(|| click_entry.arch.clone());
                                            click_state.analyze(click_entry.image.clone(), arch);
                                        },
                                    ),
                                    |_| {},
                                )
                                .child(ui_menu_item(
                                    move || SharedString::from(t(lang.get(), "recent.remove")),
                                    Some(IconName::Close),
                                    None,
                                    false,
                                    move || {
                                        remove_state.remove_recent_entry(&remove_img, &remove_arch)
                                    },
                                ))
                            })
                        },
                    )),
            ),
        )
}

fn sidebar_footer(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let appearance = state.appearance;
    let update_phase = state.update_phase;
    let palette_state = state.clone();
    let theme_state = state.clone();
    let lang_state = state.clone();
    let menu_state = state.clone();
    let update_card_state = state;

    view()
        .w_full()
        .px_3()
        .py_2p5()
        .flex_col()
        .gap_2()
        .border_t_1()
        .border_color(move || theme().sidebar_border)
        .child(dynamic(move || {
            let card_state = update_card_state.clone();
            match update_phase.get() {
                UpdatePhase::Available { version, .. } => {
                    let s_install = card_state.clone();
                    let s_skip = card_state;
                    view()
                        .w_full()
                        .p_2()
                        .rounded(radius_inset)
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().accent.opacity(0.65))
                        .flex_col()
                        .gap_1p5()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_1()
                                .child(
                                    text(format!("{} → {version}", app_version_badge()))
                                        .text_xs()
                                        .font_family(font_mono_family())
                                        .font_weight(FontWeight::MEDIUM)
                                        .number_of_lines(1)
                                        .text_color(move || theme().primary),
                                )
                                .child(ui_icon_button(
                                    IconName::Close,
                                    "Skip this version",
                                    Some(move || {
                                        SharedString::from(t(lang.get(), "update.action_skip"))
                                    }),
                                    ButtonVariant::Ghost,
                                    UiSize::Xs,
                                    false,
                                    false,
                                    move || s_skip.skip_available_update(),
                                )),
                        )
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "update.action_install")),
                            ButtonVariant::Default,
                            UiSize::Xs,
                            Some(IconName::Download),
                            false,
                            false,
                            move || s_install.start_update(),
                        ))
                        .into_element()
                }
                UpdatePhase::Downloading {
                    version,
                    received,
                    total,
                } => {
                    let progress = if total > 0 {
                        let pct = ((received as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
                        format!("{pct:.0}%")
                    } else {
                        format_bytes(received)
                    };
                    let ratio = if total > 0 {
                        (received as f32 / total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    view()
                        .w_full()
                        .p_2()
                        .rounded(radius_inset)
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().muted)
                        .flex_col()
                        .gap_1p5()
                        .child(
                            text(move || {
                                SharedString::from(tf(
                                    lang.get(),
                                    "update.downloading",
                                    &[&version, &progress],
                                ))
                            })
                            .text_xs()
                            .number_of_lines(1)
                            .text_color(move || theme().foreground),
                        )
                        .child(ui_progress_bar(
                            ratio,
                            total == 0,
                            "Update download progress",
                        ))
                        .into_element()
                }
                UpdatePhase::Installing { version } => view()
                    .w_full()
                    .p_2()
                    .rounded(radius_inset)
                    .border_1()
                    .border_color(move || theme().border)
                    .bg(move || theme().muted)
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .child(ui_spinner(px(12.0), move || theme().primary))
                    .child(
                        text(move || {
                            SharedString::from(tf(lang.get(), "update.installing", &[&version]))
                        })
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .number_of_lines(1)
                        .text_color(move || theme().foreground),
                    )
                    .into_element(),
                UpdatePhase::ReadyToRestart { version, .. } => {
                    let s_restart = card_state;
                    view()
                        .w_full()
                        .p_2()
                        .rounded(radius_inset)
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().accent.opacity(0.65))
                        .flex_col()
                        .gap_1p5()
                        .child(
                            text(move || {
                                SharedString::from(tf(
                                    lang.get(),
                                    "update.ready_restart",
                                    &[&version],
                                ))
                            })
                            .text_xs()
                            .text_color(move || theme().foreground),
                        )
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "update.restart_now")),
                            ButtonVariant::Default,
                            UiSize::Xs,
                            Some(IconName::Refresh),
                            false,
                            false,
                            move || s_restart.relaunch_after_update(),
                        ))
                        .into_element()
                }
                _ => view().into_element(),
            }
        }))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(ui_icon(IconName::Logo, px(16.0), move || theme().primary))
                        .child(
                            text(move || t(lang.get(), "app.title"))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(app_version_badge())
                                .text_xs()
                                .font_family(font_mono_family())
                                .text_color(text_tertiary),
                        ),
                )
                .child(more_actions_menu(menu_state, UiSize::Xs)),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(ui_icon_button(
                            IconName::Search,
                            "Open Command Palette",
                            Some(move || SharedString::from(t(lang.get(), "tooltip.open_palette"))),
                            ButtonVariant::Ghost,
                            UiSize::Xs,
                            false,
                            false,
                            move || palette_state.toggle_command_palette(),
                        ))
                        .child(dynamic(move || {
                            let cycle = theme_state.clone();
                            let icon = appearance_icon(appearance.get());
                            ui_icon_button(
                                icon,
                                "Switch color theme",
                                Some(move || {
                                    SharedString::from(t(lang.get(), "theme.cycle_tooltip"))
                                }),
                                ButtonVariant::Ghost,
                                UiSize::Xs,
                                false,
                                false,
                                move || cycle.cycle_appearance(),
                            )
                        })),
                )
                .child(ui_segmented(
                    vec![
                        SegmentOption::new(UiLang::En, "EN"),
                        SegmentOption::new(UiLang::Zh, "中"),
                    ],
                    lang,
                    UiSize::Xs,
                    "Display language",
                    move |next| lang_state.set_lang(next),
                )),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use diving_core::DockerAnalyzeResult;
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::domain::DivingService;
    use crate::ui::app_tokens;

    #[component]
    fn SidebarRecentHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.route.set(AppRoute::Inspector);
        state.current_image.set("redis:alpine".to_owned());
        state.arch_input.set(Some(SharedString::from("arm64")));
        let res = DockerAnalyzeResult {
            arch: "arm64".to_owned(),
            ..Default::default()
        };
        state.result.set(Some(Arc::new(res)));
        state.recent_images.set(vec![
            RecentImageEntry {
                image: "redis:alpine".to_owned(),
                arch: "amd64".to_owned(),
                score: 98,
                total_size: 1024 * 1024,
                analyzed_at: "2026-11-22 10:00".to_owned(),
                stale: true,
            },
            RecentImageEntry {
                image: "redis:alpine".to_owned(),
                arch: "arm64".to_owned(),
                score: 99,
                total_size: 2 * 1024 * 1024,
                analyzed_at: "2026-11-22 10:01".to_owned(),
                stale: true,
            },
        ]);
        app_sidebar(state)
    }

    #[test]
    fn sidebar_recent_rows_relocalize_and_match_arch_distinctly() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(400.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<SidebarRecentHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start sidebar recent harness");
        app.wait_for_within("Open OCI folder…", Duration::from_secs(3))
            .expect("folder action visible in sidebar");
        assert!(app.shows("amd64 · 1.0 MiB · stale"));
        assert!(app.shows("arm64 · 2.0 MiB · stale"));
        assert_eq!(app.screen().selected("role:button[/98%/]"), Ok(false));
        assert_eq!(app.screen().selected("role:button[/99%/]"), Ok(true));

        app.press("中").expect("switch sidebar lang to ZH");
        app.wait_for_within("打开 OCI 目录…", Duration::from_secs(3))
            .expect("folder action localized");
        app.wait_for_within("arm64 · 2.0 MiB · 快照", Duration::from_secs(3))
            .expect("stale badge in recent row reactively relocalized");
    }

    #[component]
    fn SidebarUpdaterHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.update_phase.set(UpdatePhase::Available {
            version: "v9.9.9".to_owned(),
            notes: "Updater UI check".to_owned(),
        });
        let dl_state = state.clone();
        view()
            .flex_col()
            .child(ui_button(
                "TriggerDownload",
                ButtonVariant::Ghost,
                UiSize::Xs,
                None,
                false,
                false,
                move || {
                    dl_state.update_phase.set(UpdatePhase::Downloading {
                        version: "v9.9.9".to_owned(),
                        received: 512,
                        total: 1024,
                    });
                },
            ))
            .child(app_sidebar(state))
    }

    #[test]
    fn sidebar_updater_ui_shows_available_nudge_downloading_and_menu_actions() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(400.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<SidebarUpdaterHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start sidebar updater harness");
        app.wait_for_within("Update & restart", Duration::from_secs(3))
            .expect("Update & restart nudge visible");
        assert!(app.shows(&format!("{} → v9.9.9", app_version_badge())));

        app.press("label:More actions").expect("open ... menu");
        app.wait_for_within("role:menuitem[Skip this version]", Duration::from_secs(3))
            .expect("Skip this version item visible in ... menu");
        assert!(app.shows("Check for updates…"));
        app.press_key("escape").expect("close ... menu");

        app.press("中").expect("switch sidebar lang to ZH");
        app.wait_for_within("更新并重启", Duration::from_secs(3))
            .expect("ZH updater nudge visible");

        app.press("TriggerDownload")
            .expect("switch to downloading state");
        app.wait_for_within("正在下载 v9.9.9（50%）…", Duration::from_secs(3))
            .expect("ZH live download progress shown");
    }
}
