//! Custom 48pt fusion title bar reading dynamic `title_bar()` window insets.

use nsh_core::element::desktop::title_bar;
use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState};
use crate::features::arch_select_items;
use crate::i18n::{UiLang, t};
use crate::ui::{
    BadgeTone, ButtonVariant, IconName, SegmentOption, UiSize, theme, ui_badge,
    ui_brand_home_button, ui_button, ui_icon, ui_icon_button, ui_input_with_ref, ui_menu,
    ui_menu_item, ui_menu_separator, ui_segmented, ui_select,
};

fn appearance_icon(appearance: Appearance) -> IconName {
    match appearance {
        Appearance::System => IconName::Monitor,
        Appearance::Light => IconName::Sun,
        Appearance::Dark => IconName::Moon,
    }
}

/// Renders the custom desktop title bar with safe insets and drag-region etiquette.
pub fn desktop_title_bar(state: AppState) -> impl IntoElement {
    let left_state = state.clone();
    let center_state = state.clone();
    let right_state = state.clone();

    view()
        .h(px(48.0))
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .pl(move || px(title_bar().left.0.max(16.0)))
        .pr(move || px(title_bar().right.0.max(16.0)))
        .bg(move || theme().card)
        .border_b_1()
        .border_color(move || theme().border)
        .window_drag(true)
        .child(title_bar_left(left_state))
        .child(title_bar_center(center_state))
        .child(title_bar_right(right_state))
}

fn title_bar_left(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let route = state.route;

    view()
        .flex_row()
        .items_center()
        .gap_2()
        .flex_shrink_0()
        .window_drag(false)
        .child(dynamic(move || {
            let nav_state = state.clone();
            if route.get() == AppRoute::Landing {
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(ui_icon(IconName::Logo, px(20.0), move || theme().primary))
                    .child(
                        text(move || t(lang.get(), "app.title"))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(ui_badge(
                        move || SharedString::from(t(lang.get(), "app.version_badge")),
                        BadgeTone::Neutral,
                    ))
                    .into_element()
            } else {
                ui_brand_home_button(
                    move || SharedString::from(t(lang.get(), "app.title")),
                    move || nav_state.navigate_home(),
                )
                .into_element()
            }
        }))
}

fn title_bar_center(state: AppState) -> impl IntoElement {
    let route = state.route;

    view().flex_1().min_w(px(0.0)).items_center().child(when(
        move || matches!(route.get(), AppRoute::Inspector | AppRoute::Diff),
        move || compact_search_bar(state.clone()),
    ))
}

fn compact_search_bar(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let submit_input_state = state.clone();
    let file_state = state.clone();
    let submit_btn_state = state.clone();

    view()
        .w_full()
        .max_w(px(540.0))
        .min_w(px(260.0))
        .flex_row()
        .items_center()
        .gap_1p5()
        .window_drag(false)
        .child(view().flex_1().min_w(px(140.0)).child(ui_input_with_ref(
            state.image_input,
            move || t(lang.get(), "landing.placeholder").to_owned(),
            "Compact image reference input",
            UiSize::Md,
            Some(&state.search_input_ref),
            true,
            Some(Box::new(move |_| submit_input_state.submit_current_input())),
        )))
        .child(view().w(px(116.0)).flex_shrink_0().child(ui_select(
            move || arch_select_items(lang.get()),
            state.arch_input,
            move || SharedString::from(t(lang.get(), "landing.arch_auto")),
            "Compact target architecture",
            UiSize::Md,
            state.is_loading,
            move |next| state.arch_input.set(next.filter(|s| !s.is_empty())),
        )))
        .child(ui_icon_button(
            IconName::FolderInput,
            "Open local image file",
            Some("Open local file…"),
            ButtonVariant::Outline,
            UiSize::Md,
            false,
            state.is_loading,
            move || file_state.open_local_file_dialog(),
        ))
        .child(ui_button(
            move || {
                SharedString::from(if submit_btn_state.is_loading.get() {
                    t(lang.get(), "landing.analyzing")
                } else {
                    t(lang.get(), "landing.analyze")
                })
            },
            ButtonVariant::Primary,
            UiSize::Md,
            None,
            false,
            state.is_loading,
            move || submit_btn_state.submit_current_input(),
        ))
}

fn title_bar_right(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let route = state.route;
    let appearance = state.appearance;
    let theme_state = state.clone();
    let lang_state = state.clone();
    let open_state = state.clone();
    let local_state = state.clone();
    let cmp_btn_state = state.clone();
    let cmp_menu_state = state.clone();
    let palette_state = state.clone();
    let export_state = state.clone();
    let clean_state = state.clone();
    let gallery_state = state.clone();
    let has_result = state.result;
    let diff_result = state.diff_result;

    view()
        .flex_row()
        .items_center()
        .gap_2()
        .flex_shrink_0()
        .window_drag(false)
        .child(when(
            move || route.get() == AppRoute::Inspector,
            move || {
                let s = cmp_btn_state.clone();
                ui_button(
                    move || SharedString::from(t(lang.get(), "diff.compare_with")),
                    ButtonVariant::Outline,
                    UiSize::Sm,
                    Some(IconName::Compare),
                    false,
                    false,
                    move || s.open_compare_dialog(),
                )
            },
        ))
        .child(ui_icon_button(
            IconName::Search,
            "Open Command Palette",
            Some("Command Palette (⌘K)"),
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
                Some(t(lang.get_untracked(), "theme.cycle_tooltip")),
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
        .child(
            ui_menu(ui_icon_button(
                IconName::More,
                "More actions",
                Some(t(lang.get_untracked(), "menu.more_actions")),
                ButtonVariant::Ghost,
                UiSize::Sm,
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
            )),
        )
}
