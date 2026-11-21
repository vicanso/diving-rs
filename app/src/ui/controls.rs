//! Styled checkboxes, switches, selects, and segmented controls built over `nsh_base`.

use std::rc::Rc;

use nsh_base::{SelectEntry, SelectItem, checkbox, select, switch, tab, tabs, theme};
use nsh_core::element::IntoProp;
use nsh_core::prelude::*;

use crate::ui::button::UiSize;
use crate::ui::icons::{IconName, ui_icon};
use crate::ui::motion::{FAST, INSTANT, dur};
use crate::ui::overlay::popup_surface;
use crate::ui::theme::{border_strong, radius_control, radius_inset};

/// Option descriptor for segmented controls.
#[derive(Clone)]
pub struct SegmentOption<T: Clone + PartialEq + 'static> {
    pub value: T,
    pub label: Reactive<SharedString>,
}

impl<T: Clone + PartialEq + 'static> SegmentOption<T> {
    pub fn new(value: T, label: impl IntoReactive<SharedString>) -> Self {
        Self {
            value,
            label: label.into_reactive(),
        }
    }
}

/// Styled checkbox supporting checked, unchecked, and indeterminate states.
pub fn ui_checkbox(
    label: impl IntoReactive<SharedString>,
    checked: impl IntoReactive<bool>,
    indeterminate: impl IntoReactive<bool>,
    disabled: impl IntoReactive<bool>,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    let checked = checked.into_reactive();
    let indeterminate = indeterminate.into_reactive();
    let disabled = disabled.into_reactive();
    let mark_checked = checked.clone();
    let mark_ind = indeterminate.clone();

    checkbox()
        .checked(move || checked.get())
        .indeterminate(move || indeterminate.get())
        .disabled(move || disabled.get())
        .on_change(move |next: &bool| on_change(*next))
        .flex_row()
        .items_center()
        .gap_2()
        .py_1()
        .rounded(radius_inset)
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.5))
        .indicator_style(|s| {
            s.size(px(16.0))
                .items_center()
                .justify_center()
                .rounded(radius_inset)
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().card)
                .transition_colors()
                .duration(move || dur(INSTANT))
                .checked(|s| {
                    s.bg(move || theme().primary)
                        .border_color(move || theme().primary)
                })
                .indeterminate(|s| {
                    s.bg(move || theme().primary)
                        .border_color(move || theme().primary)
                })
        })
        .indicator(
            text(move || {
                if mark_ind.get() {
                    "−"
                } else if mark_checked.get() {
                    "✓"
                } else {
                    ""
                }
            })
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(move || theme().primary_foreground),
        )
        .child(text(label).text_sm().text_color(move || theme().foreground))
}

/// Styled on/off switch with smooth thumb slide (respecting reduced motion).
pub fn ui_switch(
    label: impl IntoReactive<SharedString>,
    checked: impl IntoReactive<bool>,
    disabled: impl IntoReactive<bool>,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    let checked = checked.into_reactive();
    let disabled = disabled.into_reactive();
    switch()
        .checked(move || checked.get())
        .disabled(move || disabled.get())
        .on_change(move |next: &bool| on_change(*next))
        .flex_row()
        .items_center()
        .gap_2()
        .py_1()
        .rounded(radius_control)
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.5))
        .track_style(|s| {
            s.w(px(36.0))
                .h(px(20.0))
                .p(px(2.0))
                .rounded_full()
                .bg(move || theme().secondary)
                .border_1()
                .border_color(move || theme().border)
                .transition_colors()
                .duration(move || dur(FAST))
                .checked(|s| {
                    s.bg(move || theme().primary)
                        .border_color(move || theme().primary)
                })
        })
        .thumb_style(|s| {
            s.size(px(14.0))
                .rounded_full()
                .bg(move || theme().card)
                .transition_transform()
                .duration(move || dur(FAST))
                .ease_in_out()
                .checked(|s| s.translate_x(px(16.0)))
        })
        .child(text(label).text_sm().text_color(move || theme().foreground))
}

#[allow(clippy::too_many_arguments)]
fn build_select<M>(
    items: impl IntoProp<Vec<SelectItem>, M>,
    value: impl IntoReactive<Option<SharedString>>,
    placeholder: impl IntoReactive<SharedString>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    disabled: impl IntoReactive<bool>,
    bordered: bool,
    on_change: impl Fn(Option<SharedString>) + 'static,
) -> impl IntoElement {
    let items = items.into_prop();
    let value = value.into_reactive();
    let placeholder = placeholder.into_reactive();
    let disabled = disabled.into_reactive();

    let display_items = items.clone();
    let display_value = value.clone();
    let display_placeholder = placeholder.clone();

    let trigger_label = move || -> SharedString {
        if let Some(selected_val) = display_value.get() {
            let list = display_items.get();
            if let Some(found) = list.iter().find(|item| item.value == selected_val) {
                return found.title.clone();
            }
            return selected_val;
        }
        display_placeholder.get()
    };

    let trigger = view()
        .flex_row()
        .items_center()
        .justify_between()
        .min_h(size.min_height())
        .px(size.horizontal_padding())
        .gap(size.gap())
        .rounded(move || size.radius())
        .transition_colors()
        .duration(move || dur(INSTANT))
        .when(bordered, |el| {
            el.bg(move || theme().card)
                .border_1()
                .border_color(move || theme().input)
                .hover(|s| s.bg(move || theme().accent).border_color(border_strong))
        })
        .when(!bordered, |el| {
            el.bg(transparent_black)
                .hover(|s| s.bg(move || theme().accent))
        })
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.5))
        .child(
            text(trigger_label)
                .text_size(size.font_size())
                .number_of_lines(1)
                .text_color(move || theme().foreground),
        )
        .child(ui_icon(
            IconName::ChevronDown,
            size.icon_size(),
            move || theme().muted_foreground,
        ));

    select()
        .items(move || items.get())
        .value(move || value.get())
        .disabled(move || disabled.get())
        .accessibility_label(accessibility_label.into())
        .on_change(move |next: &Option<SharedString>| on_change(next.clone()))
        .offset(px(4.0))
        .popup_style(popup_surface)
        .popup_starting_style(|s| s.opacity(0.0).scale(0.97).translate_y(px(-4.0)))
        .popup_ending_style(|s| s.opacity(0.0).scale(0.97))
        .item_class("rounded-sm focus:bg-accent selected:bg-accent selected:font-medium disabled:opacity-45")
        .render_item(|item: &SelectEntry| {
            view()
                .px_2p5()
                .py_1p5()
                .child(
                    text(item.title.clone())
                        .text_sm()
                        .text_color(move || theme().popover_foreground),
                )
        })
        .trigger(trigger)
}

/// Styled dropdown selector sharing trigger geometry with `ui_input` and `ui_button`.
pub fn ui_select<M>(
    items: impl IntoProp<Vec<SelectItem>, M>,
    value: impl IntoReactive<Option<SharedString>>,
    placeholder: impl IntoReactive<SharedString>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    disabled: impl IntoReactive<bool>,
    on_change: impl Fn(Option<SharedString>) + 'static,
) -> impl IntoElement {
    build_select(
        items,
        value,
        placeholder,
        accessibility_label,
        size,
        disabled,
        true,
        on_change,
    )
}

/// Borderless transparent selector for unified composite command bars.
pub fn ui_borderless_select<M>(
    items: impl IntoProp<Vec<SelectItem>, M>,
    value: impl IntoReactive<Option<SharedString>>,
    placeholder: impl IntoReactive<SharedString>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    disabled: impl IntoReactive<bool>,
    on_change: impl Fn(Option<SharedString>) + 'static,
) -> impl IntoElement {
    build_select(
        items,
        value,
        placeholder,
        accessibility_label,
        size,
        disabled,
        false,
        on_change,
    )
}

/// Compact segmented control with shared outer silhouette and single sliding highlight indicator.
pub fn ui_segmented<T: Clone + PartialEq + 'static>(
    options: Vec<SegmentOption<T>>,
    current: impl IntoReactive<T>,
    size: UiSize,
    accessibility_label: impl Into<SharedString>,
    on_change: impl Fn(T) + 'static,
) -> impl IntoElement {
    let current = current.into_reactive();
    let on_change = Rc::new(on_change);

    tabs()
        .accessibility_label(accessibility_label.into())
        .flex_row()
        .items_center()
        .p(px(2.0))
        .gap_0p5()
        .rounded(move || size.radius())
        .bg(move || theme().muted)
        .indicator_style(|s| {
            s.top(px(2.0))
                .bottom(px(2.0))
                .rounded(radius_inset)
                .bg(move || theme().card)
                .shadow_xs()
        })
        .children(options.into_iter().map(move |opt| {
            let value = opt.value;
            let cur_sel = current.clone();
            let cur_txt = current.clone();
            let val_txt = value.clone();
            let val_press = value.clone();
            let on_change = on_change.clone();
            tab()
                .selected(move || cur_sel.get() == value)
                .on_press(move || on_change(val_press.clone()))
                .min_h(px((size.min_height().0 - 4.0).max(18.0)))
                .px_2p5()
                .flex_row()
                .items_center()
                .justify_center()
                .rounded(radius_inset)
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .child(
                    text(opt.label)
                        .text_size(size.font_size())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if cur_txt.get() == val_txt {
                                theme().foreground
                            } else {
                                theme().muted_foreground
                            }
                        }),
                )
        }))
}
