//! Styled buttons, icon buttons, and external links built over `nsh_base`.

use nsh_base::{ButtonElement, button, link, theme};
use nsh_core::prelude::*;

use crate::ui::icons::{IconName, ui_icon};
use crate::ui::motion::{FAST, PRESS_BTN_SCALE, PRESS_ICON_SCALE, SOFT_SETTLE, dur, press_scale};
use crate::ui::overlay::ui_tooltip;
use crate::ui::theme::{
    border_strong, destructive_active, destructive_foreground, destructive_hover, primary_active,
    primary_hover, radius_control, radius_inset, secondary_active, secondary_hover,
};

/// Unified control scale synchronized across buttons, inputs, and selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
}

impl UiSize {
    pub fn min_height(self) -> Points {
        match self {
            Self::Xs => px(20.0),
            Self::Sm => px(24.0),
            Self::Md => px(32.0),
            Self::Lg => px(40.0),
        }
    }

    pub fn icon_size(self) -> Points {
        match self {
            Self::Xs => px(12.0),
            Self::Sm => px(14.0),
            Self::Md | Self::Lg => px(16.0),
        }
    }

    pub fn font_size(self) -> Points {
        match self {
            Self::Xs | Self::Sm => px(12.0),
            Self::Md | Self::Lg => px(14.0),
        }
    }

    pub fn horizontal_padding(self) -> Points {
        match self {
            Self::Xs => px(6.0),
            Self::Sm => px(10.0),
            Self::Md => px(12.0),
            Self::Lg => px(16.0),
        }
    }

    pub fn gap(self) -> Points {
        match self {
            Self::Xs | Self::Sm => px(4.0),
            Self::Md | Self::Lg => px(8.0),
        }
    }

    pub fn radius(self) -> Points {
        match self {
            Self::Xs => radius_inset(),
            Self::Sm | Self::Md | Self::Lg => radius_control(),
        }
    }
}

/// Visual role of an application button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    /// Reserved for the single default commit in a decision region.
    Primary,
    /// Ordinary toolbar or secondary action.
    #[default]
    Default,
    /// Secondary action with clear border boundary.
    Outline,
    /// Quiet toolbar or inline action.
    Ghost,
    /// Irreversible destructive action confirmation.
    Danger,
}

fn button_fg(variant: ButtonVariant) -> AdaptiveColor {
    match variant {
        ButtonVariant::Primary => theme().primary_foreground,
        ButtonVariant::Default => theme().secondary_foreground,
        ButtonVariant::Outline | ButtonVariant::Ghost => theme().foreground,
        ButtonVariant::Danger => destructive_foreground(),
    }
}

fn apply_variant_styles(el: ButtonElement, variant: ButtonVariant) -> ButtonElement {
    match variant {
        ButtonVariant::Primary => el
            .bg(move || theme().primary)
            .hover(|s| s.bg(primary_hover))
            .active(|s| press_scale(s.bg(primary_active), PRESS_BTN_SCALE)),
        ButtonVariant::Default => el
            .bg(move || theme().secondary)
            .border_1()
            .border_color(move || theme().border)
            .hover(|s| s.bg(secondary_hover))
            .active(|s| press_scale(s.bg(secondary_active), PRESS_BTN_SCALE)),
        ButtonVariant::Outline => el
            .bg(move || theme().card)
            .border_1()
            .border_color(move || theme().border)
            .hover(|s| s.bg(move || theme().accent).border_color(border_strong))
            .active(|s| press_scale(s.bg(secondary_hover), PRESS_BTN_SCALE)),
        ButtonVariant::Ghost => el
            .hover(|s| s.bg(move || theme().accent))
            .active(|s| press_scale(s.bg(secondary_hover), PRESS_BTN_SCALE)),
        ButtonVariant::Danger => el
            .bg(move || theme().destructive)
            .hover(|s| s.bg(destructive_hover))
            .active(|s| press_scale(s.bg(destructive_active), PRESS_BTN_SCALE)),
    }
}

/// Renders a styled application button with optional leading icon, loading indicator, and focus ring.
pub fn ui_button(
    label: impl IntoReactive<SharedString>,
    variant: ButtonVariant,
    size: UiSize,
    icon: Option<IconName>,
    disabled: impl IntoReactive<bool>,
    loading: impl IntoReactive<bool>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let disabled = disabled.into_reactive();
    let loading = loading.into_reactive();
    let is_disabled = {
        let disabled = disabled.clone();
        let loading = loading.clone();
        move || disabled.get() || loading.get()
    };
    let label = label.into_reactive();
    let base = button()
        .disabled(is_disabled.clone())
        .on_press(on_press)
        .min_h(size.min_height())
        .px(size.horizontal_padding())
        .py_1()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap(size.gap())
        .rounded(move || size.radius())
        .transition_all()
        .duration(move || dur(FAST))
        .ease_bezier(SOFT_SETTLE.0, SOFT_SETTLE.1, SOFT_SETTLE.2, SOFT_SETTLE.3)
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.5))
        .style_when(loading, |s| s.opacity(0.75))
        .when_some(icon, |b, icon_name| {
            b.child(ui_icon(icon_name, size.icon_size(), move || {
                button_fg(variant)
            }))
        })
        .child(
            text(label)
                .text_size(size.font_size())
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || button_fg(variant)),
        );
    apply_variant_styles(base, variant)
}

/// Renders a Ghost brand button keeping `theme().primary` logo tint in secondary routes.
pub fn ui_brand_home_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let label = label.into_reactive();
    button()
        .on_press(on_press)
        .min_h(px(30.0))
        .px_2()
        .py_1()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .rounded(radius_control)
        .transition_all()
        .duration(move || dur(FAST))
        .ease_bezier(SOFT_SETTLE.0, SOFT_SETTLE.1, SOFT_SETTLE.2, SOFT_SETTLE.3)
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| press_scale(s.bg(secondary_hover), PRESS_BTN_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(ui_icon(IconName::Logo, px(20.0), move || theme().primary))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
}

/// Renders a square icon-only button requiring an accessible name and optional tooltip.
#[allow(clippy::too_many_arguments)]
pub fn ui_icon_button(
    icon: IconName,
    accessibility_label: impl Into<SharedString>,
    tooltip_text: Option<impl IntoReactive<SharedString>>,
    variant: ButtonVariant,
    size: UiSize,
    selected: impl IntoReactive<bool>,
    disabled: impl IntoReactive<bool>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let selected = selected.into_reactive();
    let disabled = disabled.into_reactive();
    let frame = size.min_height();
    let base = button()
        .accessibility_label(accessibility_label.into())
        .selected(move || selected.get())
        .disabled(move || disabled.get())
        .on_press(on_press)
        .size(frame)
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(move || size.radius())
        .transition_all()
        .duration(move || dur(FAST))
        .ease_bezier(SOFT_SETTLE.0, SOFT_SETTLE.1, SOFT_SETTLE.2, SOFT_SETTLE.3)
        .selected_style(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(ui_icon(icon, size.icon_size(), move || button_fg(variant)));

    let styled = match variant {
        ButtonVariant::Primary => base
            .bg(move || theme().primary)
            .hover(|s| s.bg(primary_hover))
            .active(|s| press_scale(s.bg(primary_active), PRESS_ICON_SCALE)),
        ButtonVariant::Default => base
            .bg(move || theme().secondary)
            .border_1()
            .border_color(move || theme().border)
            .hover(|s| s.bg(secondary_hover))
            .active(|s| press_scale(s.bg(secondary_active), PRESS_ICON_SCALE)),
        ButtonVariant::Outline => base
            .bg(move || theme().card)
            .border_1()
            .border_color(move || theme().border)
            .hover(|s| s.bg(move || theme().accent).border_color(border_strong))
            .active(|s| press_scale(s.bg(secondary_hover), PRESS_ICON_SCALE)),
        ButtonVariant::Ghost => base
            .hover(|s| s.bg(move || theme().accent))
            .active(|s| press_scale(s.bg(secondary_hover), PRESS_ICON_SCALE)),
        ButtonVariant::Danger => base
            .bg(move || theme().destructive)
            .hover(|s| s.bg(destructive_hover))
            .active(|s| press_scale(s.bg(destructive_active), PRESS_ICON_SCALE)),
    };

    view()
        .when_some(tooltip_text, |el, hint| el.tooltip(ui_tooltip(hint)))
        .child(styled)
}

/// Renders an external resource link (the only control allowed to use pointing-hand cursor).
pub fn ui_link(
    label: impl IntoReactive<SharedString>,
    href: impl IntoReactive<SharedString>,
    size: UiSize,
) -> impl IntoElement {
    let href = href.into_reactive();
    link()
        .href(move || Some(href.get()))
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(radius_inset)
        .cursor_pointer()
        .hover(|s| s.opacity(0.8))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(ui_icon(IconName::Link, size.icon_size(), move || {
            theme().primary
        }))
        .child(
            text(label)
                .text_size(size.font_size())
                .font_weight(FontWeight::MEDIUM)
                .underline()
                .text_color(move || theme().primary),
        )
}
