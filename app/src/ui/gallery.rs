//! Interactive `UiGallery` root showcasing all `crate::ui::*` primitives and desktop chrome.

use nsh_core::element::desktop::title_bar;
use nsh_core::prelude::*;

use crate::domain::FileKind;
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, FileOpKind, IconName, LayerOpcodeKind, SegmentOption,
    SelectItem, TabOption, UiSize, font_mono_family, register_fonts, theme, ui_badge, ui_banner,
    ui_button, ui_checkbox, ui_code_viewer, ui_context_menu, ui_dialog, ui_diff_viewer,
    ui_empty_state, ui_field, ui_file_icon, ui_icon, ui_icon_button, ui_input, ui_link, ui_menu,
    ui_menu_item, ui_menu_separator, ui_menu_trigger_icon, ui_opcode_pill, ui_panel, ui_popover,
    ui_progress_bar, ui_resizable_split, ui_search_input, ui_segmented, ui_select, ui_separator,
    ui_sheet, ui_size_bar, ui_skeleton, ui_status_badge, ui_switch, ui_tabs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    fn appearance(self) -> Appearance {
        match self {
            Self::System => Appearance::System,
            Self::Light => Appearance::Light,
            Self::Dark => Appearance::Dark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GalleryTab {
    Layers,
    Treemap,
    Findings,
    Dockerfile,
}

impl GalleryTab {
    fn label(self) -> &'static str {
        match self {
            Self::Layers => "Layers",
            Self::Treemap => "Treemap",
            Self::Findings => "Findings",
            Self::Dockerfile => "Dockerfile",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileFilterMode {
    All,
    Added,
    Modified,
}

const SAMPLE_DOCKERFILE: &str = r#"FROM alpine:3.20 AS runtime
RUN apk add --no-cache ca-certificates tzdata \
    && adduser -D -H appuser
COPY --chown=appuser:appuser dist/app /usr/local/bin/app
USER appuser
ENTRYPOINT ["/usr/local/bin/app"]
"#;

const OLD_DOCKERFILE: &str = r#"FROM ubuntu:24.04
RUN apt-get update && apt-get install -y curl vim
COPY target/debug/app /app
CMD ["/app"]
"#;

/// Component showcase mounted in M1 before wiring real analysis screens.
#[component]
pub fn UiGallery() -> impl IntoElement {
    register_fonts();
    let theme_choice = signal(ThemeChoice::System);
    let clicks = signal(0_u32);
    let dialog_open = signal(false);
    let sheet_open = signal(false);

    let image_ref = signal(String::from("redis:7-alpine"));
    let search_query = signal(String::new());
    let mirror_error_input = signal(String::from("http://insecure.local"));
    let verify_dup = signal(true);
    let follow_symlinks = signal(false);
    let selected_arch = signal(Some(SharedString::from("amd64")));
    let filter_mode = signal(FileFilterMode::All);
    let active_tab = signal(GalleryTab::Layers);
    let split_left = signal(px(220.0));

    view()
        .relative()
        .flex_1()
        .flex_col()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(gallery_title_bar(
            theme_choice,
            dialog_open,
            sheet_open,
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .w_full()
                    .items_center()
                    .child(
                        view()
                            .w_full()
                            .max_w(px(1160.0))
                            .p_6()
                            .gap_6()
                            .child(buttons_section(clicks))
                            .child(inputs_section(
                                image_ref,
                                search_query,
                                mirror_error_input,
                                verify_dup,
                                follow_symlinks,
                                selected_arch,
                                filter_mode,
                            ))
                            .child(data_section(active_tab))
                            .child(code_section(split_left)),
                    ),
            ),
        )
        .child(ui_dialog(
            dialog_open,
            "Inspection Modal",
            px(460.0),
            move || {
                view()
                    .flex_col()
                    .gap_2()
                    .child(
                        text("Sample inspection modal")
                            .text_sm()
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text("Centered modal dialog with Escape handling and theme-adaptive hairline chrome.")
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
                    .into_element()
            },
            move || {
                view()
                    .flex_row()
                    .gap_2()
                    .child(ui_button(
                        "Cancel",
                        ButtonVariant::Outline,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        move || dialog_open.set(false),
                    ))
                    .child(ui_button(
                        "Done",
                        ButtonVariant::Primary,
                        UiSize::Sm,
                        Some(IconName::Check),
                        false,
                        false,
                        move || dialog_open.set(false),
                    ))
                    .into_element()
            },
        ))
        .child(ui_sheet(
            sheet_open,
            "File Preview Sheet",
            px(420.0),
            move || {
                view()
                    .flex_1()
                    .flex_col()
                    .gap_3()
                    .child(
                        text("File preview inspection sheet")
                            .text_sm()
                            .text_color(move || theme().foreground),
                    )
                    .child(ui_code_viewer(
                        Some(SharedString::from(SAMPLE_DOCKERFILE)),
                        Some(SharedString::from("Dockerfile")),
                        true,
                        "Sheet Dockerfile snippet",
                    ))
                    .into_element()
            },
        ))
}

fn gallery_title_bar(
    theme_choice: Signal<ThemeChoice>,
    dialog_open: Signal<bool>,
    sheet_open: Signal<bool>,
) -> impl IntoElement {
    view()
        .h(px(48.0))
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .pl(move || px(title_bar().left.0.max(16.0)))
        .pr(move || px(title_bar().right.0.max(16.0)))
        .bg(move || theme().card)
        .border_b_1()
        .border_color(move || theme().border)
        .window_drag(true)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2p5()
                .window_drag(false)
                .child(ui_icon(IconName::Logo, px(20.0), move || theme().primary))
                .child(
                    text("Diving")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(ui_badge("UiGallery", BadgeTone::Neutral)),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .window_drag(false)
                .child(ui_segmented(
                    vec![
                        SegmentOption::new(ThemeChoice::System, "System"),
                        SegmentOption::new(ThemeChoice::Light, "Light"),
                        SegmentOption::new(ThemeChoice::Dark, "Dark"),
                    ],
                    theme_choice,
                    UiSize::Sm,
                    "Appearance mode",
                    move |choice| {
                        theme_choice.set(choice);
                        set_color_scheme(choice.appearance());
                    },
                ))
                .child(ui_button(
                    "Open Dialog…",
                    ButtonVariant::Outline,
                    UiSize::Sm,
                    None,
                    false,
                    false,
                    move || dialog_open.set(true),
                ))
                .child(ui_button(
                    "Open Sheet…",
                    ButtonVariant::Outline,
                    UiSize::Sm,
                    None,
                    false,
                    false,
                    move || sheet_open.set(true),
                ))
                .child(
                    ui_menu(ui_menu_trigger_icon(
                        IconName::More,
                        "More actions",
                        Some("More actions"),
                        ButtonVariant::Ghost,
                        UiSize::Sm,
                        false,
                    ))
                    .child(ui_menu_item(
                        "Open Dialog…",
                        Some(IconName::Command),
                        Some("mod+k"),
                        false,
                        move || dialog_open.set(true),
                    ))
                    .child(ui_menu_separator())
                    .child(ui_menu_item(
                        "Open Sheet…",
                        Some(IconName::Eye),
                        None,
                        false,
                        move || sheet_open.set(true),
                    )),
                ),
        )
}

fn buttons_section(clicks: Signal<u32>) -> impl IntoElement {
    let icon_selected = signal(false);

    ui_panel(
        Some("1. Buttons, Links & Overlays".into()),
        Some(
            text(move || format!("Clicks: {}", clicks.get()))
                .text_xs()
                .text_color(move || theme().muted_foreground)
                .into_element(),
        ),
        view()
            .flex_col()
            .gap_4()
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2p5()
                    .child(ui_button(
                        "Primary",
                        ButtonVariant::Primary,
                        UiSize::Md,
                        Some(IconName::Search),
                        false,
                        false,
                        move || clicks.update(|n| *n += 1),
                    ))
                    .child(ui_button(
                        "Default",
                        ButtonVariant::Default,
                        UiSize::Md,
                        Some(IconName::Layers),
                        false,
                        false,
                        || {},
                    ))
                    .child(ui_button(
                        "Outline",
                        ButtonVariant::Outline,
                        UiSize::Md,
                        Some(IconName::Compare),
                        false,
                        false,
                        || {},
                    ))
                    .child(ui_button(
                        "Ghost",
                        ButtonVariant::Ghost,
                        UiSize::Md,
                        Some(IconName::Refresh),
                        false,
                        false,
                        || {},
                    ))
                    .child(ui_button(
                        "Danger",
                        ButtonVariant::Danger,
                        UiSize::Md,
                        Some(IconName::Alert),
                        false,
                        false,
                        || {},
                    )),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2p5()
                    .child(ui_button(
                        "Xs",
                        ButtonVariant::Outline,
                        UiSize::Xs,
                        None,
                        false,
                        false,
                        || {},
                    ))
                    .child(ui_button(
                        "Sm",
                        ButtonVariant::Outline,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        || {},
                    ))
                    .child(ui_button(
                        "Md",
                        ButtonVariant::Outline,
                        UiSize::Md,
                        None,
                        false,
                        false,
                        || {},
                    ))
                    .child(ui_button(
                        "Lg",
                        ButtonVariant::Outline,
                        UiSize::Lg,
                        None,
                        false,
                        false,
                        || {},
                    ))
                    .child(ui_button(
                        "Disabled",
                        ButtonVariant::Default,
                        UiSize::Md,
                        None,
                        true,
                        false,
                        || {},
                    ))
                    .child(ui_button(
                        "Loading",
                        ButtonVariant::Primary,
                        UiSize::Md,
                        None,
                        false,
                        true,
                        || {},
                    )),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(ui_icon_button(
                        IconName::Filter,
                        "Toggle filter",
                        Some("Toggle filter"),
                        ButtonVariant::Outline,
                        UiSize::Md,
                        icon_selected,
                        false,
                        move || icon_selected.update(|v| *v = !*v),
                    ))
                    .child(ui_icon_button(
                        IconName::Copy,
                        "Copy digest",
                        Some("Copy digest"),
                        ButtonVariant::Ghost,
                        UiSize::Sm,
                        false,
                        false,
                        || {},
                    ))
                    .child(ui_icon_button(
                        IconName::Download,
                        "Unavailable download",
                        None::<SharedString>,
                        ButtonVariant::Default,
                        UiSize::Sm,
                        false,
                        true,
                        || {},
                    ))
                    .child(ui_link("View docs", "https://nsh.dev", UiSize::Sm))
                    .child(ui_popover(
                        ui_button(
                            "Popover",
                            ButtonVariant::Outline,
                            UiSize::Sm,
                            Some(IconName::Eye),
                            false,
                            false,
                            || {},
                        ),
                        || {
                            view()
                                .p_2p5()
                                .gap_1()
                                .child(
                                    text("Layer Cache Metadata")
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().popover_foreground),
                                )
                                .child(
                                    text("Pinned to sha256:9f84…c12a")
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .into_element()
                        },
                    ))
                    .child(
                        ui_context_menu(
                            view()
                                .px_3()
                                .py_1p5()
                                .rounded(px(6.0))
                                .bg(move || theme().muted)
                                .border_1()
                                .border_color(move || theme().border)
                                .child(
                                    text("Right-click surface")
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                            |_| {},
                        )
                        .child(ui_menu_item(
                            "Copy path",
                            Some(IconName::Copy),
                            Some("mod+c"),
                            false,
                            || {},
                        ))
                        .child(ui_menu_item(
                            "Export layer",
                            Some(IconName::Download),
                            None,
                            false,
                            || {},
                        )),
                    ),
            ),
    )
}

#[allow(clippy::too_many_arguments)]
fn inputs_section(
    image_ref: Signal<String>,
    search_query: Signal<String>,
    mirror_error_input: Signal<String>,
    verify_dup: Signal<bool>,
    follow_symlinks: Signal<bool>,
    selected_arch: Signal<Option<SharedString>>,
    filter_mode: Signal<FileFilterMode>,
) -> impl IntoElement {
    let arch_items = vec![
        SelectItem::new("amd64", "linux/amd64"),
        SelectItem::new("arm64", "linux/arm64"),
        SelectItem::new("arm/v7", "linux/arm/v7"),
    ];

    ui_panel(
        Some("2. Inputs & Selection Controls".into()),
        None,
        view()
            .flex_col()
            .gap_4()
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_start()
                    .gap_4()
                    .child(view().flex_1().min_w(px(240.0)).child(ui_field(
                        "Image Reference",
                        Some("Docker, OCI, or file:// archive".into()),
                        || None::<SharedString>,
                        ui_input(
                            image_ref,
                            "e.g. nginx:alpine",
                            "Image reference input",
                            UiSize::Md,
                            true,
                            None,
                        ),
                    )))
                    .child(view().flex_1().min_w(px(240.0)).child(ui_field(
                        "Filter Tree",
                        Some("Substring or regex-free path match".into()),
                        || None::<SharedString>,
                        ui_search_input(
                            search_query,
                            "Filter files in layer…",
                            "Filter files input",
                            UiSize::Md,
                            Some("/"),
                            None,
                            true,
                            None,
                        ),
                    )))
                    .child(view().flex_1().min_w(px(240.0)).child(ui_field(
                        "Registry Mirror",
                        None,
                        || Some(SharedString::from("Invalid mirror URL: HTTPS required")),
                        ui_input(
                            mirror_error_input,
                            "https://mirror.example.com",
                            "Registry mirror input",
                            UiSize::Md,
                            true,
                            None,
                        ),
                    ))),
            )
            .child(ui_separator())
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_6()
                    .child(ui_checkbox(
                        "Verify duplicates (blake3)",
                        verify_dup,
                        false,
                        false,
                        move |next| verify_dup.set(next),
                    ))
                    .child(ui_checkbox("Mixed selection", false, true, false, |_| {}))
                    .child(ui_checkbox("Locked option", true, false, true, |_| {}))
                    .child(ui_switch(
                        "Show whiteout entries",
                        follow_symlinks,
                        false,
                        move |next| follow_symlinks.set(next),
                    ))
                    .child(ui_switch("Read-only mode", true, true, |_| {})),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .child(view().w(px(190.0)).child(ui_select(
                        arch_items,
                        selected_arch,
                        "Architecture",
                        "Target architecture",
                        UiSize::Md,
                        false,
                        move |next| selected_arch.set(next),
                    )))
                    .child(ui_segmented(
                        vec![
                            SegmentOption::new(FileFilterMode::All, "All"),
                            SegmentOption::new(FileFilterMode::Added, "Added"),
                            SegmentOption::new(FileFilterMode::Modified, "Modified"),
                        ],
                        filter_mode,
                        UiSize::Md,
                        "File status filter",
                        move |mode| filter_mode.set(mode),
                    )),
            ),
    )
}

fn data_section(active_tab: Signal<GalleryTab>) -> impl IntoElement {
    ui_panel(
        Some("3. Tabs & Data Primitives".into()),
        Some(
            text(move || format!("Active tab: {}", active_tab.get().label()))
                .text_xs()
                .text_color(move || theme().muted_foreground)
                .into_element(),
        ),
        view()
            .flex_col()
            .gap_4()
            .child(ui_tabs(
                vec![
                    TabOption::new(GalleryTab::Layers, "Layers", Some(IconName::Layers)),
                    TabOption::new(GalleryTab::Treemap, "Treemap", Some(IconName::Treemap)),
                    TabOption::new(GalleryTab::Findings, "Findings", Some(IconName::Findings)),
                    TabOption::new(
                        GalleryTab::Dockerfile,
                        "Dockerfile",
                        Some(IconName::Dockerfile),
                    ),
                ],
                active_tab,
                "Inspection views",
                move |tab| active_tab.set(tab),
            ))
            .child(
                view()
                    .p_3()
                    .rounded(px(6.0))
                    .bg(move || theme().muted)
                    .border_1()
                    .border_color(move || theme().border)
                    .child(
                        text(move || format!("Active tab: {}", active_tab.get().label()))
                            .text_sm()
                            .text_color(move || theme().foreground),
                    ),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(ui_badge("Neutral", BadgeTone::Neutral))
                    .child(ui_badge("Success", BadgeTone::Success))
                    .child(ui_badge("Warning", BadgeTone::Warning))
                    .child(ui_badge("Danger", BadgeTone::Danger))
                    .child(ui_badge("Info", BadgeTone::Info))
                    .child(ui_opcode_pill(LayerOpcodeKind::From))
                    .child(ui_opcode_pill(LayerOpcodeKind::Run))
                    .child(ui_opcode_pill(LayerOpcodeKind::Copy))
                    .child(ui_opcode_pill(LayerOpcodeKind::Cmd))
                    .child(ui_opcode_pill(LayerOpcodeKind::Meta))
                    .child(ui_status_badge(FileOpKind::Added))
                    .child(ui_status_badge(FileOpKind::Modified))
                    .child(ui_status_badge(FileOpKind::Removed)),
            )
            .child({
                let mut row = view().flex_row().flex_wrap().items_center().gap_3();
                for (kind, label) in [
                    (FileKind::Image, "png"),
                    (FileKind::Audio, "mp3"),
                    (FileKind::Video, "mp4"),
                    (FileKind::Code, "js"),
                    (FileKind::Config, "json"),
                    (FileKind::Text, "txt"),
                    (FileKind::Archive, "tar.gz"),
                    (FileKind::Script, "sh"),
                    (FileKind::Binary, "elf"),
                    (FileKind::Key, "pem"),
                    (FileKind::Database, "db"),
                    (FileKind::Font, "woff2"),
                    (FileKind::Dockerfile, "Dockerfile"),
                    (FileKind::Symlink, "symlink"),
                    (FileKind::Generic, "file"),
                ] {
                    row = row.child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .child(ui_file_icon(kind, px(16.0)))
                            .child(
                                text(label)
                                    .text_xs()
                                    .font_family(font_mono_family())
                                    .text_color(move || theme().muted_foreground),
                            ),
                    );
                }
                row
            })
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_6()
                    .child(
                        view()
                            .flex_col()
                            .gap_2()
                            .child(ui_size_bar(0.18, px(140.0)))
                            .child(ui_size_bar(0.52, px(140.0)))
                            .child(ui_size_bar(0.86, px(140.0))),
                    )
                    .child(
                        view()
                            .w(px(240.0))
                            .flex_col()
                            .gap_2p5()
                            .child(ui_progress_bar(0.68, false, "Layer download progress"))
                            .child(ui_progress_bar(0.0, true, "Analyzing image layers")),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(ui_skeleton(px(96.0), px(14.0)))
                            .child(ui_skeleton(px(48.0), px(14.0))),
                    ),
            )
            .child(
                view()
                    .flex_col()
                    .gap_2()
                    .child(ui_banner(
                        BannerTone::Warning,
                        "Showing cached snapshot from registry fallback (14 minutes ago).",
                        Some(
                            ui_button(
                                "Retry",
                                ButtonVariant::Outline,
                                UiSize::Xs,
                                Some(IconName::Refresh),
                                false,
                                false,
                                || {},
                            )
                            .into_element(),
                        ),
                    ))
                    .child(ui_banner(
                        BannerTone::Danger,
                        "Secret pattern matched in layer 4 environment variable.",
                        None,
                    ))
                    .child(ui_banner(
                        BannerTone::Info,
                        "Tip: Press Cmd+K to jump between recent images and views.",
                        None,
                    )),
            )
            .child(ui_empty_state(
                IconName::Shield,
                "No findings match the current filter",
                "Clear search filters or run an analysis on another image.",
                Some(
                    ui_button(
                        "Reset filters",
                        ButtonVariant::Outline,
                        UiSize::Sm,
                        None,
                        false,
                        false,
                        || {},
                    )
                    .into_element(),
                ),
            )),
    )
}

fn code_section(split_left: Signal<Points>) -> impl IntoElement {
    ui_panel(
        Some("4. Resizable Split, Code & Diff Viewers".into()),
        None,
        view()
            .flex_col()
            .gap_4()
            .child(
                view().h(px(240.0)).w_full().child(ui_resizable_split(
                    split_left.get_untracked(),
                    px(160.0),
                    px(360.0),
                    view()
                        .flex_1()
                        .p_3()
                        .gap_2()
                        .bg(move || theme().sidebar)
                        .child(
                            text("Layers")
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().sidebar_foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(ui_opcode_pill(LayerOpcodeKind::From))
                                .child(
                                    text("alpine:3.20")
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(ui_opcode_pill(LayerOpcodeKind::Run))
                                .child(
                                    text("apk add ca-certificates")
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(ui_opcode_pill(LayerOpcodeKind::Copy))
                                .child(
                                    text("dist/app -> /usr/local/bin")
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        ),
                    ui_code_viewer(
                        Some(SharedString::from(SAMPLE_DOCKERFILE)),
                        Some(SharedString::from("Dockerfile")),
                        false,
                        "Reconstructed Dockerfile viewer",
                    ),
                    move |sizes: &Vec<Points>| {
                        if let Some(first) = sizes.first().copied() {
                            split_left.set(first);
                        }
                    },
                )),
            )
            .child(view().h(px(190.0)).w_full().child(ui_diff_viewer(
                Some(SharedString::from(OLD_DOCKERFILE)),
                Some(SharedString::from(SAMPLE_DOCKERFILE)),
                Some(SharedString::from("Dockerfile")),
                true,
            ))),
    )
}
