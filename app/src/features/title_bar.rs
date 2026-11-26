//! Unified 48pt macOS Content Header Bar paired with the collapsible left App Sidebar.

use nsh_core::element::desktop::title_bar;
use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState, InspectorTab, count_layer_file_ops, findings_total_count};
use crate::features::arch_select_items;
use crate::features::sidebar::APP_SIDEBAR_WIDTH;
use crate::i18n::{UiLang, t};
#[cfg(debug_assertions)]
use crate::ui::ui_menu_separator;
use crate::ui::{
    ButtonVariant, IconName, SegmentOption, SelectItem, TabOption, UiSize, radius_control, theme,
    ui_bare_input_with_ref, ui_borderless_select, ui_button, ui_icon_button, ui_image_pill_button,
    ui_menu, ui_menu_item, ui_menu_trigger_icon, ui_segmented, ui_tabs,
};

fn appearance_icon(appearance: Appearance) -> IconName {
    match appearance {
        Appearance::System => IconName::Monitor,
        Appearance::Light => IconName::Sun,
        Appearance::Dark => IconName::Moon,
    }
}

/// Shared More Actions overflow menu rendered in the sidebar footer and collapsed header.
pub fn more_actions_menu(state: AppState, size: UiSize) -> impl IntoElement {
    let lang = state.lang;
    let route = state.route;
    let has_result = state.result;
    let diff_error = state.diff_error;
    let open_state = state.clone();
    let folder_state = state.clone();
    let local_state = state.clone();
    let cmp_menu_state = state.clone();
    let export_disabled_state = state.clone();
    let export_state = state.clone();
    let export_json_state = state.clone();
    #[cfg(debug_assertions)]
    let gallery_state = state.clone();
    let clean_state = state;

    let menu = ui_menu(ui_menu_trigger_icon(
        IconName::More,
        "More actions",
        Some(move || SharedString::from(t(lang.get(), "menu.more_actions"))),
        ButtonVariant::Ghost,
        size,
        false,
    ))
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "menu.open_local_file")),
        Some(IconName::FolderInput),
        Some("mod+o"),
        false,
        move || open_state.open_local_file_dialog(),
    ))
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "action.open_folder")),
        Some(IconName::FolderOpen),
        None,
        false,
        move || folder_state.open_local_folder(),
    ))
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "menu.browse_local_images")),
        Some(IconName::Container),
        Some("mod+shift+o"),
        false,
        move || local_state.open_local_picker(),
    ))
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "menu.compare_with")),
        Some(IconName::Compare),
        Some("mod+d"),
        false,
        move || cmp_menu_state.open_compare_dialog(),
    ))
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "menu.export_markdown")),
        Some(IconName::Download),
        Some("mod+e"),
        move || {
            !export_disabled_state.can_export_markdown()
                || (route.get() == AppRoute::Diff && diff_error.get().is_some())
        },
        move || {
            if export_state.route.get_untracked() == AppRoute::Diff {
                export_state.export_diff_markdown();
            } else {
                export_state.export_markdown_report();
            }
        },
    ))
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "menu.export_json")),
        Some(IconName::Download),
        None,
        move || has_result.get().is_none(),
        move || export_json_state.export_json_report(),
    ))
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "menu.clear_cache")),
        Some(IconName::Refresh),
        None,
        false,
        move || clean_state.clear_cache(),
    ));
    #[cfg(debug_assertions)]
    let menu = menu.child(ui_menu_separator()).child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "nav.ui_gallery")),
        Some(IconName::Eye),
        None,
        false,
        move || gallery_state.open_gallery(),
    ));
    menu
}

/// Renders the unified 48pt top header bar for the right main stage.
pub fn content_header_bar(state: AppState) -> impl IntoElement {
    let route = state.route;
    let sidebar_open = state.app_sidebar_open;
    let lang = state.lang;

    when(
        move || route.get() != AppRoute::Gallery,
        move || {
            let bar_state = state.clone();
            view()
                .h(px(48.0))
                .w_full()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .pl(move || {
                    if sidebar_open.get() {
                        px(16.0)
                    } else {
                        px(title_bar().left.0.max(14.0))
                    }
                })
                .pr(move || px(title_bar().right.0.max(14.0)))
                .bg(move || theme().background)
                .border_b_1()
                .border_color(move || {
                    if route.get() == AppRoute::Landing {
                        AdaptiveColor::from(transparent_black())
                    } else {
                        theme().border
                    }
                })
                .window_drag(true)
                .child(dynamic(move || {
                    let left_state = bar_state.clone();
                    let left_nav = when(
                        move || !sidebar_open.get(),
                        move || collapsed_left_nav(left_state.clone(), lang),
                    );

                    match route.get() {
                        AppRoute::Landing => {
                            landing_header(bar_state.clone(), left_nav).into_element()
                        }
                        AppRoute::Inspector => {
                            inspector_header(bar_state.clone(), left_nav).into_element()
                        }
                        AppRoute::Diff => diff_header(bar_state.clone(), left_nav).into_element(),
                        AppRoute::Gallery => view().into_element(),
                    }
                }))
        },
    )
}

fn collapsed_left_nav(state: AppState, lang: Signal<UiLang>) -> impl IntoElement {
    let toggle_state = state.clone();
    let new_state = state;

    view()
        .flex_row()
        .items_center()
        .gap_1()
        .flex_shrink_0()
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
        ))
}

fn collapsed_utilities(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let appearance = state.appearance;
    let palette_state = state.clone();
    let theme_state = state.clone();
    let lang_state = state.clone();
    let menu_state = state;

    view()
        .flex_row()
        .items_center()
        .gap_1()
        .flex_shrink_0()
        .window_drag(false)
        .child(ui_icon_button(
            IconName::Search,
            "Open Command Palette",
            Some(move || SharedString::from(t(lang.get(), "tooltip.open_palette"))),
            ButtonVariant::Ghost,
            UiSize::Sm,
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
                Some(move || SharedString::from(t(lang.get(), "theme.cycle_tooltip"))),
                ButtonVariant::Ghost,
                UiSize::Sm,
                false,
                false,
                move || cycle.cycle_appearance(),
            )
        }))
        .child(ui_segmented(
            vec![
                SegmentOption::new(UiLang::En, "EN"),
                SegmentOption::new(UiLang::Zh, "中"),
            ],
            lang,
            UiSize::Sm,
            "Display language",
            move |next| lang_state.set_lang(next),
        ))
        .child(more_actions_menu(menu_state, UiSize::Sm))
}

fn landing_header(state: AppState, left_nav: impl IntoElement) -> impl IntoElement {
    let sidebar_open = state.app_sidebar_open;
    let util_state = state;

    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            view()
                .flex_row()
                .items_center()
                .window_drag(false)
                .child(left_nav),
        )
        .child(when(
            move || !sidebar_open.get(),
            move || collapsed_utilities(util_state.clone()),
        ))
}

fn effective_header_width(sidebar_open: bool) -> Points {
    let total = window().width;
    if sidebar_open {
        (total - px(APP_SIDEBAR_WIDTH)).max(px(0.0))
    } else {
        total
    }
}

fn inspector_header(state: AppState, left_nav: impl IntoElement) -> impl IntoElement {
    let lang = state.lang;
    let sidebar_open = state.app_sidebar_open;
    let header_search_open = state.header_search_open;
    let has_result = state.result;
    let is_loading = state.is_loading;

    let cmp_state = state.clone();
    let refresh_state = state.clone();
    let refresh_disabled_state = state.clone();
    let export_state = state.clone();
    let export_disabled_state = state.clone();
    let util_state = state.clone();
    let inline_state = state.clone();
    let pill_state = state.clone();
    let tabs_state = state;
    let compact = computed(move || effective_header_width(sidebar_open.get()) < px(1120.0));
    let tight = computed(move || effective_header_width(sidebar_open.get()) < px(980.0));

    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_between()
        .gap(move || if compact.get() { px(8.0) } else { px(12.0) })
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .min_w(px(0.0))
                .style_when(move || !header_search_open.get(), |s| s.flex_shrink(1.0))
                .style_when(header_search_open, |s| s.flex_1().max_w(px(460.0)))
                .window_drag(false)
                .child(left_nav)
                .child(dynamic(move || {
                    if header_search_open.get() {
                        inline_header_search_bar(inline_state.clone()).into_element()
                    } else {
                        let open_search = pill_state.clone();
                        let cur_img = pill_state.current_image;
                        let cur_res = pill_state.result;
                        view()
                            .min_w(px(0.0))
                            .flex_shrink(1.0)
                            .overflow_hidden()
                            .style_when(move || tight.get(), |s| s.max_w(px(120.0)))
                            .style_when(
                                move || compact.get() && !tight.get(),
                                |s| s.max_w(px(136.0)),
                            )
                            .child(ui_image_pill_button(
                                move || {
                                    let name = cur_img.get();
                                    if name.is_empty() {
                                        SharedString::from(t(
                                            lang.get(),
                                            "common.image_placeholder",
                                        ))
                                    } else {
                                        SharedString::from(name)
                                    }
                                },
                                move || {
                                    SharedString::from(
                                        cur_res.get().map(|r| r.arch.clone()).unwrap_or_default(),
                                    )
                                },
                                move || {
                                    SharedString::from(t(lang.get(), "header.switch_image_hint"))
                                },
                                move || open_search.open_header_search(),
                            ))
                            .into_element()
                    }
                })),
        )
        .child(when(
            move || !header_search_open.get() && has_result.get().is_some(),
            move || inspector_header_tabs(tabs_state.clone(), tight),
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .flex_shrink_0()
                .window_drag(false)
                .child(dynamic(move || {
                    if compact.get() {
                        let s = cmp_state.clone();
                        ui_icon_button(
                            IconName::Compare,
                            "Compare with…",
                            Some(move || SharedString::from(t(lang.get(), "diff.compare_with"))),
                            ButtonVariant::Ghost,
                            UiSize::Sm,
                            false,
                            false,
                            move || s.open_compare_dialog(),
                        )
                        .into_element()
                    } else {
                        let s = cmp_state.clone();
                        ui_button(
                            move || SharedString::from(t(lang.get(), "diff.compare_with")),
                            ButtonVariant::Outline,
                            UiSize::Sm,
                            Some(IconName::Compare),
                            false,
                            false,
                            move || s.open_compare_dialog(),
                        )
                        .into_element()
                    }
                }))
                .child(ui_icon_button(
                    IconName::Refresh,
                    "Re-analyze current image",
                    Some(move || SharedString::from(t(lang.get(), "cmd.reanalyze"))),
                    ButtonVariant::Ghost,
                    UiSize::Sm,
                    false,
                    move || is_loading.get() || !refresh_disabled_state.can_reanalyze(),
                    move || refresh_state.reanalyze_current_image(),
                ))
                .child(ui_icon_button(
                    IconName::Download,
                    "Export Markdown report",
                    Some(move || SharedString::from(t(lang.get(), "menu.export_markdown"))),
                    ButtonVariant::Ghost,
                    UiSize::Sm,
                    false,
                    move || !export_disabled_state.can_export_markdown(),
                    move || export_state.export_markdown_report(),
                ))
                .child(when(
                    move || !sidebar_open.get(),
                    move || collapsed_utilities(util_state.clone()),
                )),
        )
}

fn inline_header_search_bar(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let submit_input = state.clone();
    let submit_btn = state.clone();
    let close_btn = state.clone();

    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded(radius_control)
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().ring)
        .child(
            view()
                .flex_1()
                .min_w(px(120.0))
                .child(ui_bare_input_with_ref(
                    state.image_input,
                    move || t(lang.get(), "landing.placeholder").to_owned(),
                    "Header image reference input",
                    UiSize::Sm,
                    Some(&state.search_input_ref),
                    true,
                    Some(Box::new(move |_| submit_input.submit_current_input())),
                )),
        )
        .child(
            view()
                .w(px(1.0))
                .h(px(16.0))
                .bg(move || theme().border)
                .accessibility_elements_hidden(true),
        )
        .child(
            view()
                .w(px(108.0))
                .flex_shrink_0()
                .child(ui_borderless_select(
                    {
                        let res_sig = state.result;
                        move || {
                            let ui_lang = lang.get();
                            if let Some(res) = res_sig.get()
                                && res.supported_archs.len() > 1
                            {
                                let mut items = Vec::with_capacity(res.supported_archs.len() + 1);
                                items.push(SelectItem::new("", t(ui_lang, "landing.arch_auto")));
                                for arch in &res.supported_archs {
                                    items.push(SelectItem::new(arch.clone(), arch.clone()));
                                }
                                items
                            } else {
                                let mut items = arch_select_items(ui_lang);
                                if let Some(res) = res_sig.get() {
                                    for arch in &res.supported_archs {
                                        if !items.iter().any(|item| item.value.as_ref() == arch) {
                                            items.push(SelectItem::new(arch.clone(), arch.clone()));
                                        }
                                    }
                                }
                                items
                            }
                        }
                    },
                    state.arch_input,
                    move || SharedString::from(t(lang.get(), "landing.arch_auto")),
                    "Header target architecture",
                    UiSize::Sm,
                    state.is_loading,
                    move |next| state.arch_input.set(next.filter(|s| !s.is_empty())),
                )),
        )
        .child(ui_button(
            move || {
                SharedString::from(if submit_btn.is_loading.get() {
                    t(lang.get(), "landing.analyzing")
                } else {
                    t(lang.get(), "landing.analyze")
                })
            },
            ButtonVariant::Primary,
            UiSize::Xs,
            None,
            false,
            state.is_loading,
            move || submit_btn.submit_current_input(),
        ))
        .child(ui_icon_button(
            IconName::Close,
            "Close image switcher",
            Some(move || SharedString::from(t(lang.get(), "common.close_esc"))),
            ButtonVariant::Ghost,
            UiSize::Xs,
            false,
            false,
            move || close_btn.close_header_search(),
        ))
}

fn inspector_header_tabs(state: AppState, tight: Memo<bool>) -> impl IntoElement {
    let lang = state.lang;
    let active_tab = state.active_tab;
    let selected_layer = state.selected_layer;
    let result_sig = state.result;
    let summary_sig = state.summary;

    let files_tab_label = move || -> SharedString {
        let count = result_sig
            .get()
            .and_then(|res| res.file_tree_list.get(selected_layer.get()).cloned())
            .map_or(0, |tree| count_layer_file_ops(&tree).total());
        SharedString::from(format!("{} ({count})", t(lang.get(), "tab.files")))
    };

    let findings_tab_label = move || -> SharedString {
        let count = match (result_sig.get(), summary_sig.get()) {
            (Some(res), Some(sum)) => findings_total_count(&res, &sum.0),
            _ => 0,
        };
        SharedString::from(format!("{} ({count})", t(lang.get(), "tab.findings")))
    };

    view()
        .min_w(px(0.0))
        .flex_shrink(1.0)
        .window_drag(false)
        .child(dynamic(move || {
            let show_icons = !tight.get();
            ui_tabs(
                vec![
                    TabOption::new(
                        InspectorTab::FileTree,
                        files_tab_label,
                        show_icons.then_some(IconName::Layers),
                    ),
                    TabOption::new(
                        InspectorTab::Treemap,
                        move || SharedString::from(t(lang.get(), "tab.treemap")),
                        show_icons.then_some(IconName::Treemap),
                    ),
                    TabOption::new(
                        InspectorTab::Findings,
                        findings_tab_label,
                        show_icons.then_some(IconName::Findings),
                    ),
                    TabOption::new(
                        InspectorTab::Dockerfile,
                        move || SharedString::from(t(lang.get(), "tab.dockerfile")),
                        show_icons.then_some(IconName::Dockerfile),
                    ),
                ],
                active_tab,
                "Inspector workspace tabs",
                move |tab| active_tab.set(tab),
            )
        }))
}

fn diff_header(state: AppState, left_nav: impl IntoElement) -> impl IntoElement {
    let lang = state.lang;
    let sidebar_open = state.app_sidebar_open;
    let diff_result = state.diff_result;
    let diff_error = state.diff_error;
    let swap_state = state.clone();
    let open_base_state = state.clone();
    let open_target_state = state.clone();
    let cmp_state = state.clone();
    let export_state = state.clone();
    let util_state = state;
    let compact = computed(move || effective_header_width(sidebar_open.get()) < px(1120.0));

    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .min_w(px(0.0))
                .flex_shrink(1.0)
                .window_drag(false)
                .child(left_nav)
                .child(
                    text(move || t(lang.get(), "diff.title"))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .flex_shrink_0()
                .window_drag(false)
                .child(when(
                    move || diff_result.get().is_some() && diff_error.get().is_none(),
                    move || {
                        let s_swap = swap_state.clone();
                        let s_base = open_base_state.clone();
                        let s_target = open_target_state.clone();
                        let s_cmp = cmp_state.clone();
                        let s_export = export_state.clone();
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .flex_shrink_0()
                            .child(ui_button(
                                move || SharedString::from(t(lang.get(), "diff.swap")),
                                ButtonVariant::Outline,
                                UiSize::Sm,
                                Some(IconName::Swap),
                                false,
                                false,
                                move || s_swap.swap_diff(),
                            ))
                            .child(ui_button(
                                move || SharedString::from(t(lang.get(), "diff.open_base")),
                                ButtonVariant::Outline,
                                UiSize::Sm,
                                None,
                                false,
                                false,
                                move || s_base.open_diff_side(false),
                            ))
                            .child(ui_button(
                                move || SharedString::from(t(lang.get(), "diff.open_target")),
                                ButtonVariant::Outline,
                                UiSize::Sm,
                                None,
                                false,
                                false,
                                move || s_target.open_diff_side(true),
                            ))
                            .child(dynamic(move || {
                                let s = s_cmp.clone();
                                if compact.get() {
                                    ui_icon_button(
                                        IconName::Compare,
                                        "Compare with…",
                                        Some(move || {
                                            SharedString::from(t(lang.get(), "diff.compare_with"))
                                        }),
                                        ButtonVariant::Ghost,
                                        UiSize::Sm,
                                        false,
                                        false,
                                        move || s.open_compare_dialog(),
                                    )
                                    .into_element()
                                } else {
                                    ui_button(
                                        move || {
                                            SharedString::from(t(lang.get(), "diff.compare_with"))
                                        },
                                        ButtonVariant::Outline,
                                        UiSize::Sm,
                                        None,
                                        false,
                                        false,
                                        move || s.open_compare_dialog(),
                                    )
                                    .into_element()
                                }
                            }))
                            .child(dynamic(move || {
                                let s = s_export.clone();
                                if compact.get() {
                                    ui_icon_button(
                                        IconName::Download,
                                        "Export Markdown report",
                                        Some(move || {
                                            SharedString::from(t(lang.get(), "diff.export_md"))
                                        }),
                                        ButtonVariant::Primary,
                                        UiSize::Sm,
                                        false,
                                        false,
                                        move || s.export_diff_markdown(),
                                    )
                                    .into_element()
                                } else {
                                    ui_button(
                                        move || SharedString::from(t(lang.get(), "diff.export_md")),
                                        ButtonVariant::Default,
                                        UiSize::Sm,
                                        Some(IconName::Download),
                                        false,
                                        false,
                                        move || s.export_diff_markdown(),
                                    )
                                    .into_element()
                                }
                            }))
                    },
                ))
                .child(when(
                    move || !sidebar_open.get(),
                    move || collapsed_utilities(util_state.clone()),
                )),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use diving_core::{DockerAnalyzeResult, ImageDiff};
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::domain::{DivingService, SharedRef};
    use crate::features::sidebar::app_sidebar;
    use crate::ui::app_tokens;

    #[component]
    fn CollapsedHeaderHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.app_sidebar_open.set(false);
        state.route.set(AppRoute::Inspector);
        state
            .current_image
            .set("very-long-registry.example.com/org/redis:alpine-3.20".to_owned());
        state.result.set(Some(Arc::new(DockerAnalyzeResult {
            arch: "amd64".to_owned(),
            supported_archs: vec!["amd64".to_owned()],
            ..Default::default()
        })));
        state.summary.set(Some(SharedRef::new(
            DockerAnalyzeResult::default().summary(),
        )));

        let diff_state = state.clone();
        let gallery_state = state.clone();
        let search_state = state.clone();
        let sidebar_state = state.clone();

        view()
            .flex_row()
            .w_full()
            .h_full()
            .child(when(
                move || sidebar_state.app_sidebar_open.get(),
                move || app_sidebar(sidebar_state.clone()),
            ))
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex_col()
                    .child(content_header_bar(state))
                    .child(ui_button(
                        "GoDiff",
                        ButtonVariant::Ghost,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            diff_state.route.set(AppRoute::Diff);
                            diff_state
                                .diff_result
                                .set(Some(SharedRef::new(ImageDiff::default())));
                        },
                    ))
                    .child(ui_button(
                        "OpenHeaderSearch",
                        ButtonVariant::Ghost,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || {
                            search_state.route.set(AppRoute::Inspector);
                            search_state.open_header_search();
                        },
                    ))
                    .child(ui_button(
                        "GoGallery",
                        ButtonVariant::Ghost,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        move || gallery_state.open_gallery(),
                    )),
            )
    }

    fn assert_header_fits_within(app: &Headless, max_right: f32) {
        let nodes = app.screen().layout_nodes();
        let header_children: Vec<_> = nodes
            .iter()
            .filter(|n| {
                n.bounds.origin.y.0 > 0.0
                    && n.bounds.origin.y.0 + n.bounds.size.height.0 < 48.0
                    && n.bounds.size.height.0 > 0.0
                    && n.bounds.size.height.0 < 48.0
            })
            .collect();
        assert!(
            !header_children.is_empty(),
            "expected layout nodes inside 48pt header bar"
        );
        for node in header_children {
            let right = node.bounds.origin.x.0 + node.bounds.size.width.0;
            assert!(
                right <= max_right,
                "header child right edge ({right:.1}) exceeded max_right ({max_right:.1})"
            );
        }
    }

    #[test]
    fn collapsed_header_exposes_utilities_more_menu_and_hides_on_gallery() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(960.0), Points(640.0)),
            ..EngineConfig::new(windowed_root::<CollapsedHeaderHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start collapsed header harness");
        app.wait_for_within("label:New analysis", Duration::from_secs(3))
            .expect("collapsed + button on Inspector");
        assert_eq!(app.screen().matches("label:Open Command Palette"), Ok(true));
        assert_eq!(app.screen().matches("label:Switch color theme"), Ok(true));
        assert!(app.shows("EN"));

        // Verify Inspector header items stay clear of the 960pt viewport right edge.
        assert_header_fits_within(&app, 960.0 - 12.0);

        // Clicking More actions opens the popup menu cleanly.
        app.press("label:More actions")
            .expect("press More actions trigger");
        app.wait_for_within("role:menuitem[Open OCI folder…]", Duration::from_secs(3))
            .expect("More actions menu opened");
        assert_eq!(app.shows("UI Gallery"), cfg!(debug_assertions));
        app.press_key("escape").expect("close menu");

        // Open header search on a single-arch image: amd64 and arm64 remain selectable.
        app.press("OpenHeaderSearch")
            .expect("open header search bar");
        app.wait_for_within("label:Header target architecture", Duration::from_secs(3))
            .expect("header arch dropdown visible");
        app.press("label:Header target architecture")
            .expect("open header arch dropdown");
        app.wait_for_within("linux/arm64", Duration::from_secs(3))
            .expect("arm64 remains selectable on single-arch image");
        assert!(app.shows("linux/amd64"));
        app.press_key("escape").expect("close arch dropdown");
        app.press("label:Close image switcher")
            .expect("close header search");

        // Switch to Diff and verify utilities remain mounted inside 960pt viewport.
        app.press("GoDiff").expect("switch to Diff");
        app.wait_for_within("Swap", Duration::from_secs(3))
            .expect("Diff actions visible");
        assert_eq!(app.screen().matches("label:New analysis"), Ok(true));
        assert_eq!(app.screen().matches("label:Open Command Palette"), Ok(true));
        assert_eq!(app.screen().matches("label:More actions"), Ok(true));
        assert_header_fits_within(&app, 960.0 - 12.0);

        // Toggle sidebar open at 960x640 on Diff: header actions stay within viewport.
        app.press("label:Toggle sidebar")
            .expect("open sidebar at 960x640");
        app.wait_for_within("Workbench", Duration::from_secs(3))
            .expect("sidebar opened");
        assert_eq!(
            app.screen().matches("label:Export Markdown report"),
            Ok(true)
        );
        assert_header_fits_within(&app, 960.0 - 12.0);

        // Collapse sidebar again before switching to Gallery so header unmount check stays isolated.
        app.press("label:Toggle sidebar")
            .expect("close sidebar before Gallery");
        app.wait_for_gone("Workbench", Duration::from_secs(3))
            .expect("sidebar collapsed");

        // Switch to Gallery: content_header_bar unmounts to avoid double header.
        app.press("GoGallery").expect("switch to Gallery");
        app.wait_for_gone("Swap", Duration::from_secs(3))
            .expect("header collapsed on Gallery");
        assert_eq!(app.screen().matches("label:New analysis"), Ok(false));
    }
}
