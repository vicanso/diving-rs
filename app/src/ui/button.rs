//! Styled buttons, icon buttons, and external links built over `nsh_base`.

use nsh_base::{ButtonElement, button, link, theme};
use nsh_core::element::StyleRefinement;
use nsh_core::prelude::*;

use crate::ui::icons::{IconName, ui_icon, ui_spinner};
use crate::ui::motion::{FAST, PRESS_BTN_SCALE, PRESS_ICON_SCALE, SOFT_SETTLE, dur, press_scale};
use crate::ui::overlay::{format_shortcut_hint, ui_tooltip};
use crate::ui::theme::{
    border_strong, danger, destructive_active, destructive_foreground, destructive_hover,
    font_mono_family, primary_active, primary_hover, radius_control, radius_inset,
    secondary_active, secondary_hover, success, surface_active, text_tertiary, warning,
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
            Self::Xs => px(22.0),
            Self::Sm => px(26.0),
            Self::Md => px(30.0),
            Self::Lg => px(36.0),
        }
    }

    pub fn icon_size(self) -> Points {
        match self {
            Self::Xs => px(12.0),
            Self::Sm => px(14.0),
            Self::Md | Self::Lg => px(15.0),
        }
    }

    pub fn font_size(self) -> Points {
        match self {
            Self::Xs | Self::Sm => px(12.0),
            Self::Md | Self::Lg => px(13.0),
        }
    }

    pub fn horizontal_padding(self) -> Points {
        match self {
            Self::Xs => px(7.0),
            Self::Sm => px(10.0),
            Self::Md => px(12.0),
            Self::Lg => px(14.0),
        }
    }

    pub fn gap(self) -> Points {
        match self {
            Self::Xs | Self::Sm => px(4.0),
            Self::Md | Self::Lg => px(6.0),
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

trait VariantPressControl: Styled {
    fn on_hover_style(self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self;
    fn on_active_style(self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self;
}

impl VariantPressControl for ButtonElement {
    fn on_hover_style(self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.hover(f)
    }

    fn on_active_style(self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.active(f)
    }
}

impl VariantPressControl for View {
    fn on_hover_style(self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.hover(f)
    }

    fn on_active_style(self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.active(f)
    }
}

fn apply_variant_styles<T: VariantPressControl>(el: T, variant: ButtonVariant, press: f32) -> T {
    match variant {
        ButtonVariant::Primary => el
            .bg(move || theme().primary)
            .on_hover_style(|s| s.bg(primary_hover))
            .on_active_style(move |s| press_scale(s.bg(primary_active), press)),
        ButtonVariant::Default => el
            .bg(move || theme().secondary)
            .on_hover_style(|s| s.bg(secondary_hover))
            .on_active_style(move |s| press_scale(s.bg(secondary_active), press)),
        ButtonVariant::Outline => el
            .bg(move || theme().card)
            .border_1()
            .border_color(move || theme().border)
            .on_hover_style(|s| s.bg(move || theme().accent).border_color(border_strong))
            .on_active_style(move |s| press_scale(s.bg(secondary_hover), press)),
        ButtonVariant::Ghost => el
            .on_hover_style(|s| s.bg(move || theme().accent))
            .on_active_style(move |s| press_scale(s.bg(secondary_hover), press)),
        ButtonVariant::Danger => el
            .bg(move || theme().destructive)
            .on_hover_style(|s| s.bg(destructive_hover))
            .on_active_style(move |s| press_scale(s.bg(destructive_active), press)),
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
    let loading_spinner = loading.clone();
    let loading_icon = loading.clone();
    let label = label.into_reactive();
    let base = button()
        .disabled(is_disabled)
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
        .style_when(loading, |s| s.opacity(0.85))
        .child(when(
            move || loading_spinner.get(),
            move || ui_spinner(size.icon_size(), move || button_fg(variant)),
        ))
        .when_some(icon, |b, icon_name| {
            let loading_hide = loading_icon.clone();
            b.child(when(
                move || !loading_hide.get(),
                move || ui_icon(icon_name, size.icon_size(), move || button_fg(variant)),
            ))
        })
        .child(
            text(label)
                .text_size(size.font_size())
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || button_fg(variant)),
        );
    apply_variant_styles(base, variant, PRESS_BTN_SCALE)
}

/// Sidebar navigation item with subtle active pill, icon, label, and optional shortcut hint.
pub fn ui_sidebar_item(
    label: impl IntoReactive<SharedString>,
    icon: IconName,
    shortcut: Option<&'static str>,
    active: impl IntoReactive<bool>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let active = active.into_reactive();
    let icon_active = active.clone();
    let text_active = active.clone();
    let hint = shortcut.map(|s| SharedString::from(format_shortcut_hint(s)));

    button()
        .selected(move || active.get())
        .on_press(on_press)
        .w_full()
        .min_h(px(28.0))
        .px_2()
        .py_1()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .rounded(radius_inset)
        .transition_colors()
        .duration(move || dur(FAST))
        .selected_style(|s| s.bg(surface_active))
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.bg(secondary_active))
        .focus_visible(|s| {
            s.outline(px(2.0))
                .outline_color(move || theme().sidebar_ring)
        })
        .child(
            view()
                .flex_row()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap_2()
                .child(ui_icon(icon, px(14.0), move || {
                    if icon_active.get() {
                        theme().primary
                    } else {
                        theme().muted_foreground
                    }
                }))
                .child(
                    text(label)
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .number_of_lines(1)
                        .text_color(move || {
                            if text_active.get() {
                                theme().foreground
                            } else {
                                theme().sidebar_foreground
                            }
                        }),
                ),
        )
        .when_some(hint, |row, kbd| {
            row.child(
                text(kbd)
                    .text_xs()
                    .font_family(font_mono_family())
                    .text_color(text_tertiary),
            )
        })
}

/// Recent image item row rendered inside the persistent left sidebar.
pub fn ui_recent_item(
    image: SharedString,
    meta: SharedString,
    score_label: SharedString,
    score: u64,
    active: impl IntoReactive<bool>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let active = active.into_reactive();
    let score_color = move || {
        if score >= 95 {
            success()
        } else if score >= 85 {
            warning()
        } else {
            danger()
        }
    };

    button()
        .selected(move || active.get())
        .on_press(on_press)
        .w_full()
        .px_2()
        .py_1p5()
        .flex_col()
        .items_start()
        .gap_0p5()
        .rounded(radius_inset)
        .transition_colors()
        .duration(move || dur(FAST))
        .selected_style(|s| s.bg(surface_active))
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.bg(secondary_active))
        .focus_visible(|s| {
            s.outline(px(2.0))
                .outline_color(move || theme().sidebar_ring)
        })
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(image)
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .number_of_lines(1)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(score_label)
                        .flex_shrink_0()
                        .text_xs()
                        .font_family(font_mono_family())
                        .font_weight(FontWeight::MEDIUM)
                        .tabular_nums()
                        .text_color(score_color),
                ),
        )
        .when(!meta.is_empty(), |col| {
            col.child(
                text(meta)
                    .w_full()
                    .text_xs()
                    .font_family(font_mono_family())
                    .tabular_nums()
                    .number_of_lines(1)
                    .text_color(text_tertiary),
            )
        })
}

/// Tonal quick-action launcher card with icon, title, subtitle, and trailing shortcut hint.
pub fn ui_action_card(
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    icon: IconName,
    shortcut: Option<&'static str>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let hint = shortcut.map(|s| SharedString::from(format_shortcut_hint(s)));

    button()
        .on_press(on_press)
        .flex_1()
        .min_w(px(0.0))
        .p_3()
        .flex_col()
        .items_start()
        .gap_1p5()
        .rounded(radius_control)
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .transition_all()
        .duration(move || dur(FAST))
        .ease_bezier(SOFT_SETTLE.0, SOFT_SETTLE.1, SOFT_SETTLE.2, SOFT_SETTLE.3)
        .hover(|s| s.bg(move || theme().accent).border_color(border_strong))
        .active(|s| press_scale(s.bg(secondary_hover), PRESS_BTN_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui_icon(icon, px(15.0), move || theme().primary))
                .when_some(hint, |row, kbd| {
                    row.child(
                        text(kbd)
                            .text_xs()
                            .font_family(font_mono_family())
                            .text_color(text_tertiary),
                    )
                }),
        )
        .child(
            text(title)
                .w_full()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .number_of_lines(1)
                .text_color(move || theme().foreground),
        )
        .child(
            text(subtitle)
                .w_full()
                .text_xs()
                .number_of_lines(1)
                .text_color(move || theme().muted_foreground),
        )
}

/// Clickable active-image identity pill inside Inspector's 48pt header bar.
pub fn ui_image_pill_button(
    image: impl IntoReactive<SharedString>,
    arch: impl IntoReactive<SharedString>,
    tooltip_text: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let arch = arch.into_reactive();
    let arch_show = arch.clone();

    view().tooltip(ui_tooltip(tooltip_text)).child(
        button()
            .on_press(on_press)
            .min_h(px(28.0))
            .max_w(px(280.0))
            .px_2()
            .py_1()
            .flex_row()
            .items_center()
            .gap_1p5()
            .rounded(radius_control)
            .transition_colors()
            .duration(move || dur(FAST))
            .hover(|s| s.bg(move || theme().accent))
            .active(|s| press_scale(s.bg(secondary_hover), PRESS_BTN_SCALE))
            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
            .child(ui_icon(IconName::Layers, px(14.0), move || theme().primary))
            .child(
                text(image)
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .font_family(font_mono_family())
                    .font_weight(FontWeight::MEDIUM)
                    .number_of_lines(1)
                    .text_color(move || theme().foreground),
            )
            .child(when(
                move || !arch_show.get().is_empty(),
                move || {
                    text(arch.clone())
                        .flex_shrink_0()
                        .text_xs()
                        .font_family(font_mono_family())
                        .text_color(text_tertiary)
                },
            )),
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

    let styled = apply_variant_styles(base, variant, PRESS_ICON_SCALE);

    view()
        .when_some(tooltip_text, |el, hint| el.tooltip(ui_tooltip(hint)))
        .child(styled)
}

/// Renders a square icon-only trigger compatible with `ui_menu` (`nsh_base::menu().trigger(...)`).
///
/// Unlike [`ui_icon_button`], this renders a press-neutral `view()` without an inner `button()`
/// press handler so `nsh_base::menu`'s own trigger `on_press` receives clicks directly.
pub fn ui_menu_trigger_icon(
    icon: IconName,
    accessibility_label: impl Into<SharedString>,
    tooltip_text: Option<impl IntoReactive<SharedString>>,
    variant: ButtonVariant,
    size: UiSize,
    disabled: impl IntoReactive<bool>,
) -> impl IntoElement {
    let disabled = disabled.into_reactive();
    let disabled_focus = disabled.clone();
    let frame = size.min_height();
    let base = view()
        .role(AccessibilityRole::Button)
        .label(accessibility_label.into())
        .focusable(move || !disabled_focus.get())
        .disabled(move || disabled.get())
        .size(frame)
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(move || size.radius())
        .transition_all()
        .duration(move || dur(FAST))
        .ease_bezier(SOFT_SETTLE.0, SOFT_SETTLE.1, SOFT_SETTLE.2, SOFT_SETTLE.3)
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .when_some(tooltip_text, |el, hint| el.tooltip(ui_tooltip(hint)))
        .child(ui_icon(icon, size.icon_size(), move || button_fg(variant)));

    apply_variant_styles(base, variant, PRESS_ICON_SCALE)
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
