//! Styled text fields, search inputs, and labeled form rows built over `nsh_base::input`.

use nsh_base::{Input, input, theme};
use nsh_core::prelude::*;

use crate::ui::button::{ButtonVariant, UiSize, ui_icon_button};
use crate::ui::icons::{IconName, ui_icon};
use crate::ui::motion::{INSTANT, dur};
use crate::ui::theme::{border_strong, font_mono_family, radius_inset};

/// Optional submit callback passed to `ui_input`, `ui_bare_input_with_ref`, and `ui_search_input`.
pub type InputSubmit = Box<dyn Fn(&str)>;

fn build_raw_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    focus_ref: Option<&ElementRef>,
    monospace: bool,
    on_submit: Option<InputSubmit>,
) -> Input {
    let field = input()
        .when_some(focus_ref, |el, r| el.element_ref(r))
        .value(move || value.get())
        .on_change(move |next: &str| value.set(next.to_owned()))
        .placeholder(placeholder)
        .placeholder_color(|| color(theme().muted_foreground))
        .caret_color(|| color(theme().primary))
        .selection_color(|| color(theme().primary.opacity(0.24)))
        .text_color(move || theme().foreground)
        .label(accessibility_label.into())
        .w_full()
        .min_w(px(0.0))
        .text_size(size.font_size())
        .when(monospace, |el| el.font_family(font_mono_family()));

    match on_submit {
        Some(submit) => field.on_submit(move |text: &str| submit(text)),
        None => field,
    }
}

/// Styled single-line text input sharing height and horizontal padding with selects and buttons.
pub fn ui_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    monospace: bool,
    on_submit: Option<InputSubmit>,
) -> impl IntoElement {
    ui_input_with_ref(
        value,
        placeholder,
        accessibility_label,
        size,
        None,
        monospace,
        on_submit,
    )
}

/// Styled single-line text input accepting an optional `ElementRef` for programmatic focusing (`Mod+L`).
pub fn ui_input_with_ref(
    value: Signal<String>,
    placeholder: impl IntoReactive<String>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    focus_ref: Option<&ElementRef>,
    monospace: bool,
    on_submit: Option<InputSubmit>,
) -> impl IntoElement {
    build_raw_input(
        value,
        placeholder,
        accessibility_label,
        size,
        focus_ref,
        monospace,
        on_submit,
    )
    .min_h(size.min_height())
    .px(size.horizontal_padding())
    .py_1()
    .rounded(move || size.radius())
    .bg(move || theme().card)
    .border_1()
    .border_color(move || theme().input)
    .transition_colors()
    .duration(move || dur(INSTANT))
    .hover(|s| s.border_color(border_strong))
    .focus(|s| s.border_color(move || theme().ring))
    .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
}

/// Borderless text input whose surrounding composite bar owns the outer chrome and focus ring.
pub fn ui_bare_input_with_ref(
    value: Signal<String>,
    placeholder: impl IntoReactive<String>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    focus_ref: Option<&ElementRef>,
    monospace: bool,
    on_submit: Option<InputSubmit>,
) -> impl IntoElement {
    build_raw_input(
        value,
        placeholder,
        accessibility_label,
        size,
        focus_ref,
        monospace,
        on_submit,
    )
    .min_h(size.min_height())
    .px_1p5()
    .py_0p5()
}

/// Search field with leading search icon, optional shortcut badge (`/`), and clear button.
#[allow(clippy::too_many_arguments)]
pub fn ui_search_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    shortcut_hint: Option<&'static str>,
    focus_ref: Option<&ElementRef>,
    monospace: bool,
    on_submit: Option<InputSubmit>,
) -> impl IntoElement {
    let has_value = move || value.with(|v| !v.is_empty());

    let raw_input = build_raw_input(
        value,
        placeholder,
        accessibility_label,
        size,
        focus_ref,
        monospace,
        on_submit,
    );

    view()
        .flex_row()
        .items_center()
        .min_h(size.min_height())
        .px(size.horizontal_padding())
        .gap(size.gap())
        .rounded(move || size.radius())
        .bg(move || theme().muted)
        .border_1()
        .border_color(move || theme().border)
        .transition_colors()
        .duration(move || dur(INSTANT))
        .hover(|s| s.border_color(border_strong))
        .focus_within(|s| {
            s.bg(move || theme().card)
                .border_color(move || theme().ring)
        })
        .child(view().flex_shrink_0().child(ui_icon(
            IconName::Search,
            size.icon_size(),
            move || theme().muted_foreground,
        )))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .pr_1()
                .overflow_hidden()
                .child(raw_input),
        )
        .child(when(
            move || shortcut_hint.is_some() && !has_value(),
            move || {
                view()
                    .flex_shrink_0()
                    .px_1p5()
                    .py_0p5()
                    .rounded(radius_inset)
                    .bg(move || theme().card)
                    .child(
                        text(shortcut_hint.unwrap_or_default())
                            .text_xs()
                            .font_family(font_mono_family())
                            .text_color(move || theme().muted_foreground),
                    )
            },
        ))
        .child(when(has_value, move || {
            view().flex_shrink_0().child(ui_icon_button(
                IconName::Close,
                "Clear input",
                Some("Clear (Esc)"),
                ButtonVariant::Ghost,
                UiSize::Xs,
                false,
                false,
                move || value.set(String::new()),
            ))
        }))
}

/// Labeled form field placing visible label above the control and optional help/error below.
pub fn ui_field(
    label: impl IntoReactive<SharedString>,
    help: Option<SharedString>,
    error: impl IntoReactive<Option<SharedString>>,
    control: impl IntoElement,
) -> impl IntoElement {
    let error = error.into_reactive();
    let err_show = error.clone();
    let err_text = error.clone();
    let help_show = error;
    view()
        .flex_col()
        .gap_1()
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(control)
        .child(when(
            move || err_show.get().is_some(),
            move || {
                let err_inner = err_text.clone();
                text(move || err_inner.get().unwrap_or_default())
                    .text_xs()
                    .text_color(move || theme().destructive)
            },
        ))
        .when_some(help, |el, msg| {
            el.child(when(
                move || help_show.get().is_none(),
                move || {
                    text(msg.clone())
                        .text_xs()
                        .text_color(move || theme().muted_foreground)
                },
            ))
        })
}
