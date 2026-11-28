//! Unified popup surfaces, menus, context menus, tooltips, dialogs, sheets, and banners.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nsh_base::{
    CommandEntry, CommandGroup, CommandItem as BaseCommandItem, ContextMenuElement,
    ContextMenuEvents, MenuElement, MenuEvent, SheetEvent, TooltipElement, command, context_menu,
    dialog, menu, menu_item, menu_separator, popover, sheet, theme, tooltip,
};
use nsh_core::element::{IntoProp, StyleRefinement};
use nsh_core::prelude::*;

use crate::ui::button::{ButtonVariant, UiSize, ui_icon_button};
use crate::ui::icons::{IconName, ui_icon};
use crate::ui::motion::{FAST, INSTANT, dialog_enter_spec, dur};
use crate::ui::theme::{
    ambient_lang_signal, backdrop_scrim, danger, danger_soft, font_mono_family, info, info_soft,
    localized_ui_label, radius_control, radius_inset, radius_lg, success, success_soft, warning,
    warning_soft,
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

#[derive(Default)]
struct MenuOverlayTracker {
    open_menus: usize,
    next_id: u64,
    listeners: Vec<(u64, Signal<bool>)>,
}

thread_local! {
    static MENU_OVERLAY_TRACKER: RefCell<MenuOverlayTracker> =
        RefCell::new(MenuOverlayTracker::default());
}

fn adjust_open_menu_count(delta: isize) {
    MENU_OVERLAY_TRACKER.with(|cell| {
        let mut tracker = cell.borrow_mut();
        tracker.open_menus = tracker.open_menus.saturating_add_signed(delta);
        let any_open = tracker.open_menus > 0;
        tracker.listeners.retain(|(_, sig)| sig.try_set(any_open));
    });
}

/// Subscribes the current reactive owner to whether any [`ui_menu`] popup is currently open.
pub(crate) fn subscribe_any_menu_open() -> Signal<bool> {
    let initial = MENU_OVERLAY_TRACKER.with(|cell| cell.borrow().open_menus > 0);
    let open = signal(initial);
    let id = MENU_OVERLAY_TRACKER.with(|cell| {
        let mut tracker = cell.borrow_mut();
        let id = tracker.next_id;
        tracker.next_id = tracker.next_id.wrapping_add(1);
        tracker.listeners.push((id, open));
        id
    });
    cleanup(move || {
        MENU_OVERLAY_TRACKER.with(|cell| {
            cell.borrow_mut()
                .listeners
                .retain(|(entry_id, _)| *entry_id != id);
        });
    });
    open
}

/// Styled anchored menu built over `nsh_base::menu`.
pub fn ui_menu(trigger: impl IntoElement) -> MenuElement {
    let is_open = Rc::new(Cell::new(false));
    let sync_open = Rc::new(move |next: bool| {
        if is_open.replace(next) != next {
            adjust_open_menu_count(if next { 1 } else { -1 });
        }
    });
    let cleanup_sync = sync_open.clone();
    cleanup(move || cleanup_sync(false));
    let on_change_sync = sync_open.clone();
    let on_open_sync = sync_open.clone();
    let on_close_sync = sync_open;

    menu()
        .offset(px(4.0))
        .popup_style(popup_surface)
        .popup_starting_style(|s| s.opacity(0.0).scale(0.97).translate_y(px(-4.0)))
        .popup_ending_style(|s| s.opacity(0.0).scale(0.97))
        .on_open_change(move |event: &MenuEvent| {
            if let MenuEvent::OpenChange(next, _) = event {
                on_change_sync(*next);
            }
        })
        .on_open(move || on_open_sync(true))
        .on_close(move || on_close_sync(false))
        .trigger(trigger)
}

/// Styled right-click context menu using `0.7.0` `ContextMenuEvents`.
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
    let label = label.into_reactive();
    let a11y_label = label.clone();
    let disabled = disabled.into_reactive();
    let shortcut_label = shortcut.map(|s| SharedString::from(format_shortcut_hint(s)));
    menu_item()
        .accessibility_label(move || a11y_label.get())
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

/// Formats shortcut strings like `"mod+shift+o"` using native platform symbols (`⇧⌘O` / `Ctrl+Shift+O`).
pub fn format_shortcut_hint(raw: &str) -> String {
    let mut parts: Vec<&str> = raw.split('+').collect();
    let key = parts.pop().unwrap_or_default().to_ascii_uppercase();
    let has_mod = parts.iter().any(|p| p.eq_ignore_ascii_case("mod"));
    let has_shift = parts.iter().any(|p| p.eq_ignore_ascii_case("shift"));

    if apple_keys() {
        let mut out = String::new();
        if has_shift {
            out.push('⇧');
        }
        if has_mod {
            out.push('⌘');
        }
        out.push_str(&key);
        out
    } else {
        let mut out = String::new();
        if has_mod {
            out.push_str("Ctrl+");
        }
        if has_shift {
            out.push_str("Shift+");
        }
        out.push_str(&key);
        out
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
    let lang_sig = ambient_lang_signal();
    when(open, move || {
        let m = motion().entering(dialog_enter_spec());
        let label_title = title.clone();
        dialog()
            .label(move || label_title.get())
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
                                Some(move || {
                                    localized_ui_label(lang_sig, "Close (Esc)", "关闭 (Esc)")
                                }),
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
    let lang_sig = ambient_lang_signal();
    sheet()
        .open(move || Some(open.get()))
        .side(Placement::Right)
        .accessibility_label(move || Some(label_title.get()))
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
                    Some(move || localized_ui_label(lang_sig, "Close (Esc)", "关闭 (Esc)")),
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
    Success,
    Warning,
    Danger,
    Info,
}

fn banner_icon(tone: BannerTone) -> IconName {
    match tone {
        BannerTone::Success => IconName::Check,
        BannerTone::Warning | BannerTone::Danger => IconName::Alert,
        BannerTone::Info => IconName::Info,
    }
}

fn banner_fg(tone: BannerTone) -> AdaptiveColor {
    match tone {
        BannerTone::Success => success(),
        BannerTone::Warning => warning(),
        BannerTone::Danger => danger(),
        BannerTone::Info => info(),
    }
}

fn banner_bg(tone: BannerTone) -> AdaptiveColor {
    match tone {
        BannerTone::Success => success_soft(),
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
                        .flex_1()
                        .min_w(px(0.0))
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
                            .item_class("focus:bg-accent")
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
