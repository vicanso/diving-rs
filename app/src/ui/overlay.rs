//! Unified popup surfaces, menus, context menus, tooltips, dialogs, sheets, and banners.

use std::rc::Rc;

use nsh_base::{
    CommandEntry, CommandGroup, CommandItem as BaseCommandItem, ContextMenuElement,
    ContextMenuEvents, MenuElement, SheetEvent, TooltipElement, command, context_menu, dialog,
    menu, menu_item, menu_separator, popover, sheet, theme, tooltip,
};
use nsh_core::element::{IntoProp, StyleRefinement};
use nsh_core::prelude::*;

use crate::ui::button::{ButtonVariant, UiSize, ui_icon_button};
use crate::ui::icons::{IconName, ui_icon};
use crate::ui::motion::{FAST, INSTANT, dur};
use crate::ui::theme::{
    backdrop_scrim, danger, danger_soft, font_mono_family, info, info_soft, radius_control,
    radius_inset, radius_lg, warning, warning_soft,
};

/// Re-exported command entry builder for `ui_command_palette`.
pub type CommandItem = BaseCommandItem;

/// Shared surface styling applied uniformly across menus, popovers, and selects.
pub fn popup_surface(s: StyleRefinement) -> StyleRefinement {
    s.min_w(px(188.0))
        .p_1()
        .rounded(radius_control)
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .shadow_lg()
        .transition_all()
        .duration(move || dur(FAST))
}

/// Styled anchored menu built over `nsh_base::menu`.
pub fn ui_menu(trigger: impl IntoElement) -> MenuElement {
    menu()
        .offset(px(4.0))
        .popup_style(popup_surface)
        .popup_starting_style(|s| s.opacity(0.0).scale(0.97).translate_y(px(-4.0)))
        .popup_ending_style(|s| s.opacity(0.0).scale(0.97))
        .trigger(trigger)
}

/// Styled right-click context menu using `0.6.1` `ContextMenuEvents`.
pub fn ui_context_menu(
    trigger: impl IntoElement,
    on_open_change: impl Fn(&ContextMenuEvents) + 'static,
) -> ContextMenuElement {
    context_menu()
        .popup_style(popup_surface)
        .popup_starting_style(|s| s.opacity(0.0).scale(0.97))
        .popup_ending_style(|s| s.opacity(0.0).scale(0.97))
        .on_open_change(on_open_change)
        .trigger(trigger)
}

/// Styled item inside `ui_menu` or `ui_context_menu`.
pub fn ui_menu_item(
    label: impl IntoReactive<SharedString>,
    icon: Option<IconName>,
    shortcut: Option<&'static str>,
    disabled: impl IntoReactive<bool>,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    let disabled = disabled.into_reactive();
    let shortcut_label = shortcut.map(|s| SharedString::from(keystroke_label_hint(s)));
    menu_item()
        .disabled(disabled)
        .on_select(on_select)
        .px_2p5()
        .py_1p5()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .rounded(radius_inset)
        .transition_colors()
        .duration(move || dur(INSTANT))
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.bg(move || theme().accent))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .when_some(icon, |row, name| {
                    row.child(ui_icon(name, px(14.0), move || theme().muted_foreground))
                })
                .child(
                    text(label)
                        .text_sm()
                        .text_color(move || theme().popover_foreground),
                ),
        )
        .when_some(shortcut_label, |row, hint| {
            row.child(
                text(hint)
                    .text_xs()
                    .font_family(font_mono_family())
                    .text_color(move || theme().muted_foreground),
            )
        })
}

fn keystroke_label_hint(raw: &str) -> String {
    if apple_keys() {
        raw.replace("mod+", "⌘").replace("shift+", "⇧")
    } else {
        raw.replace("mod+", "Ctrl+").replace("shift+", "Shift+")
    }
}

/// Hairline separator between menu groups.
pub fn ui_menu_separator() -> impl IntoElement {
    menu_separator()
        .h(px(1.0))
        .my_1()
        .bg(move || theme().border)
}

/// Styled anchored popover built over `nsh_base::popover`.
pub fn ui_popover(
    trigger: impl IntoElement,
    content: impl Fn() -> AnyElement + 'static,
) -> impl IntoElement {
    popover()
        .offset(px(6.0))
        .margin(px(8.0))
        .popup_style(popup_surface)
        .popup_starting_style(|s| s.opacity(0.0).scale(0.97).translate_y(px(-4.0)))
        .popup_ending_style(|s| s.opacity(0.0).scale(0.97))
        .trigger(trigger)
        .child(content())
}

/// Styled hover/keyboard tooltip.
pub fn ui_tooltip(label: impl IntoReactive<SharedString>) -> TooltipElement {
    let label = label.into_reactive();
    tooltip()
        .px_2()
        .py_1()
        .rounded(radius_inset)
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .shadow_md()
        .child(
            text(move || label.get())
                .text_xs()
                .text_color(move || theme().popover_foreground),
        )
}

/// Centered desktop modal dialog with scrim, Escape handling, header, body, and footer.
pub fn ui_dialog(
    open: Signal<bool>,
    title: impl IntoReactive<SharedString>,
    max_width: Points,
    body: impl Fn() -> AnyElement + 'static,
    footer: impl Fn() -> AnyElement + 'static,
) -> impl IntoElement {
    let title = title.into_reactive();
    when(open, move || {
        let m = motion().entering(MotionPreset::FadeInUp);
        let label_title = title.clone();
        dialog()
            .label(untracked(move || label_title.get()))
            .on_cancel(move || open.set(false))
            .flex_1()
            .items_center()
            .justify_center()
            .p_6()
            .child(
                view()
                    .absolute()
                    .inset_0()
                    .bg(|| AdaptiveColor::from(backdrop_scrim()))
                    .on_press(move || open.set(false)),
            )
            .child(
                view()
                    .bind_motion(&m)
                    .w_full()
                    .max_w(max_width)
                    .rounded(radius_lg)
                    .bg(move || theme().card)
                    .border_1()
                    .border_color(move || theme().border)
                    .shadow_xl()
                    .overflow_hidden()
                    .on_press(|| {})
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(move || theme().border)
                            .child(
                                text(title.clone())
                                    .role(AccessibilityRole::Header)
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().card_foreground),
                            )
                            .child(ui_icon_button(
                                IconName::Close,
                                "Close dialog",
                                Some("Close (Esc)"),
                                ButtonVariant::Ghost,
                                UiSize::Sm,
                                false,
                                false,
                                move || open.set(false),
                            )),
                    )
                    .child(view().p_4().child(body()))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .px_4()
                            .py_3()
                            .border_t_1()
                            .border_color(move || theme().border)
                            .bg(move || theme().muted)
                            .child(footer()),
                    ),
            )
    })
}

/// Right-edge sliding inspection sheet built over `nsh_base::sheet`.
pub fn ui_sheet(
    open: Signal<bool>,
    title: impl IntoReactive<SharedString>,
    width: Points,
    body: impl Fn() -> AnyElement + 'static,
) -> impl IntoElement {
    let title = title.into_reactive();
    let label_title = title.clone();
    sheet()
        .open(move || Some(open.get()))
        .side(Placement::Right)
        .accessibility_label(untracked(move || label_title.get()))
        .on_open_change(move |ev: &SheetEvent| {
            if let SheetEvent::OpenChange(next, _) = ev {
                open.set(*next);
            }
        })
        .backdrop_style(|s| s.bg(|| AdaptiveColor::from(backdrop_scrim())))
        .popup_style(move |s| {
            s.w(width)
                .h_full()
                .flex_col()
                .bg(move || theme().card)
                .border_l_1()
                .border_color(move || theme().border)
                .shadow_2xl()
        })
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    text(title)
                        .role(AccessibilityRole::Header)
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1)
                        .text_color(move || theme().card_foreground),
                )
                .child(ui_icon_button(
                    IconName::Close,
                    "Close sheet",
                    Some("Close (Esc)"),
                    ButtonVariant::Ghost,
                    UiSize::Sm,
                    false,
                    false,
                    move || open.set(false),
                )),
        )
        .child(view().flex_1().min_h(px(0.0)).p_4().child(body()))
}

/// Inline status banner tones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerTone {
    Warning,
    Danger,
    Info,
}

fn banner_icon(tone: BannerTone) -> IconName {
    match tone {
        BannerTone::Warning | BannerTone::Danger => IconName::Alert,
        BannerTone::Info => IconName::Findings,
    }
}

fn banner_fg(tone: BannerTone) -> AdaptiveColor {
    match tone {
        BannerTone::Warning => warning(),
        BannerTone::Danger => danger(),
        BannerTone::Info => info(),
    }
}

fn banner_bg(tone: BannerTone) -> AdaptiveColor {
    match tone {
        BannerTone::Warning => warning_soft(),
        BannerTone::Danger => danger_soft(),
        BannerTone::Info => info_soft(),
    }
}

/// Renders a quiet full-width hairline status banner with icon, message, and trailing action.
pub fn ui_banner(
    tone: BannerTone,
    message: impl IntoReactive<SharedString>,
    action: Option<AnyElement>,
) -> impl IntoElement {
    let icon = banner_icon(tone);
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .px_3()
        .py_2()
        .rounded(radius_control)
        .bg(move || banner_bg(tone))
        .border_1()
        .border_color(move || banner_fg(tone).opacity(0.35))
        .child(
            view()
                .flex_row()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap_2()
                .child(ui_icon(icon, px(16.0), move || banner_fg(tone)))
                .child(
                    text(message)
                        .text_sm()
                        .text_color(move || theme().foreground),
                ),
        )
        .when_some(action, |row, act| row.child(act))
}

/// Modal global command palette wrapping `nsh_base::dialog` + `nsh_base::command()`.
pub fn ui_command_palette<M>(
    open: Signal<bool>,
    items: impl IntoProp<Vec<CommandItem>, M>,
    placeholder: impl IntoReactive<Option<SharedString>>,
    empty_label: impl IntoReactive<SharedString>,
    on_select: impl Fn(&str) + 'static,
) -> impl IntoElement {
    let items = items.into_prop();
    let placeholder = placeholder.into_reactive();
    let empty_label = empty_label.into_reactive();
    let on_select = Rc::new(on_select);

    when(open, move || {
        let items = items.clone();
        let placeholder = placeholder.clone();
        let empty_label = empty_label.clone();
        let on_select = Rc::clone(&on_select);
        let m = motion().entering(MotionPreset::FadeInUp);

        dialog()
            .label("Command Palette")
            .on_cancel(move || open.set(false))
            .flex_1()
            .items_center()
            .justify_start()
            .pt(px(88.0))
            .px_6()
            .child(
                view()
                    .absolute()
                    .inset_0()
                    .bg(|| AdaptiveColor::from(backdrop_scrim()))
                    .on_press(move || open.set(false)),
            )
            .child(
                view()
                    .bind_motion(&m)
                    .w_full()
                    .max_w(px(560.0))
                    .rounded(radius_lg)
                    .bg(move || theme().card)
                    .border_1()
                    .border_color(move || theme().border)
                    .shadow_xl()
                    .overflow_hidden()
                    .on_press(|| {})
                    .child(
                        command()
                            .items(move || items.get())
                            .placeholder(move || placeholder.get())
                            .accessibility_label("Command Palette")
                            .auto_focus(true)
                            .search_bar_style(|s| {
                                s.flex_row()
                                    .items_center()
                                    .gap_2p5()
                                    .px_3p5()
                                    .py_2p5()
                                    .border_b_1()
                                    .border_color(move || theme().border)
                            })
                            .search_style(|s| {
                                s.flex_1()
                                    .min_w(px(0.0))
                                    .text_sm()
                                    .text_color(move || theme().foreground)
                            })
                            .search_prefix(ui_icon(IconName::Search, px(16.0), move || {
                                theme().muted_foreground
                            }))
                            .list_style(|s| s.max_h(px(360.0)).p_1p5())
                            .separator_style(|s| {
                                s.h(px(1.0)).my_1().mx_2().bg(move || theme().border)
                            })
                            .item_class("rounded-sm focus:bg-accent")
                            .item_style(|s| s.px_2p5().py_1p5().rounded(radius_inset))
                            .render_group(|group: &CommandGroup| {
                                view().px_2().pt_2().pb_1().child(
                                    text(group.heading.clone())
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().muted_foreground),
                                )
                            })
                            .render_item(|entry: &CommandEntry| {
                                let disabled = entry.disabled;
                                view()
                                    .w_full()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        view()
                                            .flex_row()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .items_center()
                                            .gap_2()
                                            .when(entry.checked, |r| {
                                                r.child(ui_icon(
                                                    IconName::Check,
                                                    px(14.0),
                                                    move || theme().primary,
                                                ))
                                            })
                                            .child(
                                                text(entry.label.clone())
                                                    .text_sm()
                                                    .number_of_lines(1)
                                                    .text_color(move || {
                                                        if disabled {
                                                            theme().muted_foreground
                                                        } else {
                                                            theme().foreground
                                                        }
                                                    }),
                                            ),
                                    )
                                    .when_some(entry.shortcut_label.clone(), |r, keys| {
                                        r.child(
                                            view()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded(radius_inset)
                                                .bg(move || theme().muted)
                                                .border_1()
                                                .border_color(move || theme().border)
                                                .child(
                                                    text(keys)
                                                        .text_xs()
                                                        .font_family(font_mono_family())
                                                        .text_color(move || {
                                                            theme().muted_foreground
                                                        }),
                                                ),
                                        )
                                    })
                            })
                            .empty(
                                view().p_6().items_center().justify_center().child(
                                    text(move || empty_label.get())
                                        .text_sm()
                                        .text_color(move || theme().muted_foreground),
                                ),
                            )
                            .on_select(move |id: &SharedString| {
                                open.set(false);
                                on_select(id.as_str());
                            })
                            .on_cancel(move || open.set(false)),
                    ),
            )
    })
}
