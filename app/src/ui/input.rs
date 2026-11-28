//! Styled text fields, search inputs, and labeled form rows built over `nsh_base::input`.

use std::rc::Rc;

use nsh_base::{Input, input, theme};
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::ui::button::{ButtonVariant, UiSize, ui_icon_button};
use crate::ui::icons::{IconName, ui_icon};
use crate::ui::motion::{INSTANT, dur};
use crate::ui::theme::{
    ambient_lang_signal, border_strong, font_mono_family, localized_ui_label, radius_inset,
};

/// Optional submit callback passed to `ui_input`, `ui_bare_input_with_ref`, and `ui_search_input`.
pub type InputSubmit = Box<dyn Fn(&str)>;

/// Optional Escape callback passed to `ui_search_input_with_escape` when the field is already empty.
pub type InputEscape = Box<dyn Fn()>;

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

/// Masked secret/password input with inline show/hide visibility toggle.
pub fn ui_password_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    monospace: bool,
) -> impl IntoElement {
    let lang_sig = ambient_lang_signal();
    let revealed = signal(false);
    let raw_input = build_raw_input(
        value,
        placeholder,
        accessibility_label,
        size,
        None,
        monospace,
        None,
    )
    .masked(move || !revealed.get());

    view()
        .flex_row()
        .items_center()
        .min_h(size.min_height())
        .pl(size.horizontal_padding())
        .pr_1()
        .gap_1()
        .rounded(move || size.radius())
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().input)
        .transition_colors()
        .duration(move || dur(INSTANT))
        .hover(|s| s.border_color(border_strong))
        .focus_within(|s| s.border_color(move || theme().ring))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(raw_input),
        )
        .child(view().flex_shrink_0().child(ui_icon_button(
            IconName::Eye,
            "Toggle secret visibility",
            Some(move || {
                if revealed.get() {
                    localized_ui_label(lang_sig, "Hide secret", "隐藏内容")
                } else {
                    localized_ui_label(lang_sig, "Reveal secret", "显示内容")
                }
            }),
            ButtonVariant::Ghost,
            UiSize::Xs,
            false,
            false,
            move || revealed.set(!revealed.get_untracked()),
        )))
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
    ui_search_input_with_escape(
        value,
        placeholder,
        accessibility_label,
        size,
        shortcut_hint,
        focus_ref,
        monospace,
        on_submit,
        None,
    )
}

/// Search field with `Esc` handling: clears non-empty text (and blurs, or calls `on_escape` when empty).
#[allow(clippy::too_many_arguments)]
pub fn ui_search_input_with_escape(
    value: Signal<String>,
    placeholder: impl IntoReactive<String>,
    accessibility_label: impl Into<SharedString>,
    size: UiSize,
    shortcut_hint: Option<&'static str>,
    focus_ref: Option<&ElementRef>,
    monospace: bool,
    on_submit: Option<InputSubmit>,
    on_escape: Option<InputEscape>,
) -> impl IntoElement {
    let lang_sig = ambient_lang_signal();
    let has_text = computed(move || value.with(|v| !v.is_empty()));
    let has_value = move || has_text.get();
    let focused = signal(false);
    let local_ref = element_ref();
    let input_ref = focus_ref.copied().unwrap_or(local_ref);

    let raw_input = build_raw_input(
        value,
        placeholder,
        accessibility_label,
        size,
        Some(&input_ref),
        monospace,
        on_submit,
    )
    .on_focus(move || {
        let _ = focused.try_set(true);
    })
    .on_blur(move || {
        let _ = focused.try_set(false);
    });

    let esc_cb = on_escape.map(Rc::new);
    effect(move || {
        if !focused.get() {
            return;
        }
        let non_empty = has_text.get();
        let esc_cb = esc_cb.clone();
        if non_empty || esc_cb.is_some() {
            on_key_with(
                "escape",
                KeyOptions {
                    when: When::Always,
                    repeat: false,
                    ..Default::default()
                },
                move |_| {
                    if !value.with_untracked(|v| v.is_empty()) {
                        value.set(String::new());
                        if esc_cb.is_none() {
                            input_ref.blur();
                        }
                    } else if let Some(cb) = &esc_cb {
                        cb();
                    }
                },
            );
        } else {
            on_key_with(
                "escape",
                KeyOptions {
                    when: When::Always,
                    repeat: false,
                    pass_through: true,
                    ..Default::default()
                },
                move |_| input_ref.blur(),
            );
        }
    });

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
                Some(move || localized_ui_label(lang_sig, "Clear (Esc)", "清除 (Esc)")),
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
