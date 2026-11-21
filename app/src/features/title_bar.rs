//! Unified 48pt macOS Content Header Bar paired with the collapsible left App Sidebar.

use nsh_core::element::desktop::title_bar;
use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState, InspectorTab, count_layer_file_ops, findings_total_count};
use crate::features::arch_select_items;
use crate::i18n::{UiLang, t};
use crate::ui::{
    ButtonVariant, IconName, SegmentOption, TabOption, UiSize, radius_control, theme,
    ui_bare_input_with_ref, ui_borderless_select, ui_button, ui_icon_button, ui_image_pill_button,
    ui_menu, ui_menu_item, ui_menu_separator, ui_segmented, ui_tabs,
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
    let diff_result = state.diff_result;
    let open_state = state.clone();
    let local_state = state.clone();
    let cmp_menu_state = state.clone();
    let export_state = state.clone();
    let clean_state = state.clone();
    let gallery_state = state;

    ui_menu(ui_icon_button(
        IconName::More,
        "More actions",
        Some(move || SharedString::from(t(lang.get(), "menu.more_actions"))),
        ButtonVariant::Ghost,
        size,
        false,
        false,
        || {},
    ))
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "menu.open_local_file")),
        Some(IconName::FolderInput),
        Some("mod+o"),
        false,
        move || open_state.open_local_file_dialog(),
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
            if route.get() == AppRoute::Diff {
                diff_result.get().is_none()
            } else {
                has_result.get().is_none()
            }
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
        move || SharedString::from(t(lang.get(), "menu.clear_cache")),
        Some(IconName::Refresh),
        None,
        false,
        move || clean_state.clear_cache(),
    ))
    .child(ui_menu_separator())
    .child(ui_menu_item(
        move || SharedString::from(t(lang.get(), "nav.ui_gallery")),
        Some(IconName::Eye),
        None,
        false,
        move || gallery_state.open_gallery(),
    ))
}

/// Renders the unified 48pt top header bar for the right main stage.
pub fn content_header_bar(state: AppState) -> impl IntoElement {
    let route = state.route;
    let sidebar_open = state.app_sidebar_open;
    let lang = state.lang;

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
            let toggle_state = state.clone();
            let toggle_btn = when(
                move || !sidebar_open.get(),
                move || {
                    let s = toggle_state.clone();
                    ui_icon_button(
                        IconName::PanelLeft,
                        "Toggle sidebar",
                        Some(move || SharedString::from(t(lang.get(), "sidebar.toggle_tooltip"))),
                        ButtonVariant::Ghost,
                        UiSize::Sm,
                        false,
                        false,
                        move || s.toggle_app_sidebar(),
                    )
                },
            );

            match route.get() {
                AppRoute::Landing => landing_header(state.clone(), toggle_btn).into_element(),
                AppRoute::Inspector => inspector_header(state.clone(), toggle_btn).into_element(),
                AppRoute::Diff => diff_header(state.clone(), toggle_btn).into_element(),
                AppRoute::Gallery => gallery_header(state.clone(), toggle_btn).into_element(),
            }
        }))
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
        .gap_1p5()
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

fn landing_header(state: AppState, toggle_btn: impl IntoElement) -> impl IntoElement {
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
                .child(toggle_btn),
        )
        .child(when(
            move || !sidebar_open.get(),
            move || collapsed_utilities(util_state.clone()),
        ))
}

fn inspector_header(state: AppState, toggle_btn: impl IntoElement) -> impl IntoElement {
    let lang = state.lang;
    let sidebar_open = state.app_sidebar_open;
    let header_search_open = state.header_search_open;
    let has_result = state.result;
    let is_loading = state.is_loading;

    let cmp_state = state.clone();
    let refresh_state = state.clone();
    let export_state = state.clone();
    let menu_state = state.clone();
    let inline_state = state.clone();
    let pill_state = state.clone();
    let tabs_state = state;
    let compact = computed(move || window().width < px(1120.0));

    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .min_w(px(0.0))
                .when(header_search_open.get(), |el| el.flex_1().max_w(px(460.0)))
                .window_drag(false)
                .child(toggle_btn)
                .child(dynamic(move || {
                    if header_search_open.get() {
                        inline_header_search_bar(inline_state.clone()).into_element()
                    } else {
                        let open_search = pill_state.clone();
                        let cur_img = pill_state.current_image;
                        let cur_res = pill_state.result;
                        ui_image_pill_button(
                            move || {
                                let name = cur_img.get();
                                if name.is_empty() {
                                    SharedString::from("Image")
                                } else {
                                    SharedString::from(name)
                                }
                            },
                            move || {
                                SharedString::from(
                                    cur_res.get().map(|r| r.arch.clone()).unwrap_or_default(),
                                )
                            },
                            move || SharedString::from(t(lang.get(), "header.switch_image_hint")),
                            move || open_search.open_header_search(),
                        )
                        .into_element()
                    }
                })),
        )
        .child(when(
            move || !header_search_open.get() && has_result.get().is_some(),
            move || inspector_header_tabs(tabs_state.clone()),
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
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
                    is_loading,
                    move || refresh_state.reanalyze_current_image(),
                ))
                .child(ui_icon_button(
                    IconName::Download,
                    "Export Markdown report",
                    Some(move || SharedString::from(t(lang.get(), "menu.export_markdown"))),
                    ButtonVariant::Ghost,
                    UiSize::Sm,
                    false,
                    move || has_result.get().is_none(),
                    move || export_state.export_markdown_report(),
                ))
                .child(when(
                    move || !sidebar_open.get(),
                    move || more_actions_menu(menu_state.clone(), UiSize::Sm),
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
                    move || arch_select_items(lang.get()),
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
            Some("Close (Esc)"),
            ButtonVariant::Ghost,
            UiSize::Xs,
            false,
            false,
            move || close_btn.close_header_search(),
        ))
}

fn inspector_header_tabs(state: AppState) -> impl IntoElement {
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

    view().flex_shrink_0().window_drag(false).child(ui_tabs(
        vec![
            TabOption::new(
                InspectorTab::FileTree,
                files_tab_label,
                Some(IconName::Layers),
            ),
            TabOption::new(
                InspectorTab::Treemap,
                move || SharedString::from(t(lang.get(), "tab.treemap")),
                Some(IconName::Treemap),
            ),
            TabOption::new(
                InspectorTab::Findings,
                findings_tab_label,
                Some(IconName::Findings),
            ),
            TabOption::new(
                InspectorTab::Dockerfile,
                move || SharedString::from(t(lang.get(), "tab.dockerfile")),
                Some(IconName::Dockerfile),
            ),
        ],
        active_tab,
        "Inspector workspace tabs",
        move |tab| active_tab.set(tab),
    ))
}

fn diff_header(state: AppState, toggle_btn: impl IntoElement) -> impl IntoElement {
    let lang = state.lang;
    let diff_result = state.diff_result;
    let swap_state = state.clone();
    let open_base_state = state.clone();
    let open_target_state = state.clone();
    let cmp_state = state.clone();
    let export_state = state;

    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .window_drag(false)
                .child(toggle_btn)
                .child(
                    text(move || t(lang.get(), "diff.title"))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(when(
            move || diff_result.get().is_some(),
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
                    .window_drag(false)
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
                    .child(ui_button(
                        move || SharedString::from(t(lang.get(), "diff.compare_with")),
                        ButtonVariant::Outline,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        move || s_cmp.open_compare_dialog(),
                    ))
                    .child(ui_button(
                        move || SharedString::from(t(lang.get(), "diff.export_md")),
                        ButtonVariant::Default,
                        UiSize::Sm,
                        Some(IconName::Download),
                        false,
                        false,
                        move || s_export.export_diff_markdown(),
                    ))
            },
        ))
}

fn gallery_header(state: AppState, toggle_btn: impl IntoElement) -> impl IntoElement {
    let lang = state.lang;
    let back_state = state;

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
                .gap_2()
                .window_drag(false)
                .child(toggle_btn)
                .child(ui_button(
                    move || SharedString::from(t(lang.get(), "nav.back_home")),
                    ButtonVariant::Ghost,
                    UiSize::Sm,
                    None,
                    false,
                    false,
                    move || back_state.navigate_home(),
                ))
                .child(
                    text(move || t(lang.get(), "nav.ui_gallery"))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                ),
        )
}
