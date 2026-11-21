//! Styled workspace tabs built over `nsh_base::{tabs, tab}` with native sliding indicator.

use std::rc::Rc;

use nsh_base::{tab, tabs, theme};
use nsh_core::prelude::*;

use crate::ui::icons::{IconName, ui_icon};
use crate::ui::motion::{INSTANT, dur};
use crate::ui::theme::{radius_control, radius_inset};

/// Descriptor for one workspace tab.
#[derive(Clone)]
pub struct TabOption<T: Clone + PartialEq + 'static> {
    pub value: T,
    pub label: Reactive<SharedString>,
    pub icon: Option<IconName>,
}

impl<T: Clone + PartialEq + 'static> TabOption<T> {
    pub fn new(value: T, label: impl IntoReactive<SharedString>, icon: Option<IconName>) -> Self {
        Self {
            value,
            label: label.into_reactive(),
            icon,
        }
    }
}

/// Renders a horizontal workspace tab bar whose indicator slides natively between selections.
pub fn ui_tabs<T: Clone + PartialEq + 'static>(
    items: Vec<TabOption<T>>,
    current: impl IntoReactive<T>,
    accessibility_label: impl Into<SharedString>,
    on_change: impl Fn(T) + 'static,
) -> impl IntoElement {
    let current = current.into_reactive();
    let on_change = Rc::new(on_change);

    tabs()
        .accessibility_label(accessibility_label.into())
        .flex_row()
        .items_center()
        .p(px(2.5))
        .gap_0p5()
        .rounded(radius_control)
        .bg(move || theme().muted)
        .indicator_style(|s| {
            s.top(px(2.5))
                .bottom(px(2.5))
                .rounded(radius_inset)
                .bg(move || theme().card)
                .shadow_xs()
        })
        .children(items.into_iter().map(move |item| {
            let value = item.value;
            let val_sel = value.clone();
            let val_txt = value.clone();
            let val_ico = value.clone();
            let cur_sel = current.clone();
            let cur_txt = current.clone();
            let cur_ico = current.clone();
            let on_change = on_change.clone();

            tab()
                .selected(move || cur_sel.get() == val_sel)
                .on_press(move || on_change(value.clone()))
                .min_h(px(25.0))
                .px_2p5()
                .py_0p5()
                .flex_row()
                .items_center()
                .gap_1p5()
                .rounded(radius_inset)
                .transition_colors()
                .duration(move || dur(INSTANT))
                .hover(|s| s.opacity(0.85))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .when_some(item.icon, move |el, icon_name| {
                    let cur = cur_ico.clone();
                    let val = val_ico.clone();
                    el.child(ui_icon(icon_name, px(13.0), move || {
                        if cur.get() == val {
                            theme().primary
                        } else {
                            theme().muted_foreground
                        }
                    }))
                })
                .child(
                    text(item.label)
                        .text_xs()
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
