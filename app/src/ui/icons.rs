//! Single-family SVG icons rendered natively via `nsh_core::prelude::svg`.

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

/// Identifiers for every UI icon bundled in `app/assets/icons/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconName {
    Logo,
    Search,
    Folder,
    FolderOpen,
    File,
    Link,
    Layers,
    Treemap,
    Findings,
    Dockerfile,
    Alert,
    Shield,
    Copy,
    Check,
    Sun,
    Moon,
    Monitor,
    Globe,
    Compare,
    Swap,
    Download,
    FolderInput,
    Container,
    ChevronRight,
    ChevronDown,
    Close,
    More,
    Refresh,
    Command,
    Filter,
    Eye,
}

impl IconName {
    fn markup(self) -> &'static str {
        match self {
            Self::Logo => include_str!("../../assets/icons/logo.svg"),
            Self::Search => include_str!("../../assets/icons/search.svg"),
            Self::Folder => include_str!("../../assets/icons/folder.svg"),
            Self::FolderOpen => include_str!("../../assets/icons/folder-open.svg"),
            Self::File => include_str!("../../assets/icons/file.svg"),
            Self::Link => include_str!("../../assets/icons/link.svg"),
            Self::Layers => include_str!("../../assets/icons/layers.svg"),
            Self::Treemap => include_str!("../../assets/icons/treemap.svg"),
            Self::Findings => include_str!("../../assets/icons/findings.svg"),
            Self::Dockerfile => include_str!("../../assets/icons/dockerfile.svg"),
            Self::Alert => include_str!("../../assets/icons/alert.svg"),
            Self::Shield => include_str!("../../assets/icons/shield.svg"),
            Self::Copy => include_str!("../../assets/icons/copy.svg"),
            Self::Check => include_str!("../../assets/icons/check.svg"),
            Self::Sun => include_str!("../../assets/icons/sun.svg"),
            Self::Moon => include_str!("../../assets/icons/moon.svg"),
            Self::Monitor => include_str!("../../assets/icons/monitor.svg"),
            Self::Globe => include_str!("../../assets/icons/globe.svg"),
            Self::Compare => include_str!("../../assets/icons/compare.svg"),
            Self::Swap => include_str!("../../assets/icons/swap.svg"),
            Self::Download => include_str!("../../assets/icons/download.svg"),
            Self::FolderInput => include_str!("../../assets/icons/folder-input.svg"),
            Self::Container => include_str!("../../assets/icons/container.svg"),
            Self::ChevronRight => include_str!("../../assets/icons/chevron-right.svg"),
            Self::ChevronDown => include_str!("../../assets/icons/chevron-down.svg"),
            Self::Close => include_str!("../../assets/icons/close.svg"),
            Self::More => include_str!("../../assets/icons/more.svg"),
            Self::Refresh => include_str!("../../assets/icons/refresh.svg"),
            Self::Command => include_str!("../../assets/icons/command.svg"),
            Self::Filter => include_str!("../../assets/icons/filter.svg"),
            Self::Eye => include_str!("../../assets/icons/eye.svg"),
        }
    }
}

/// Renders an icon centered in a fixed square slot so adjacent labels never shift.
pub fn ui_icon(
    name: IconName,
    size_pt: Points,
    color: impl IntoReactive<AdaptiveColor>,
) -> impl IntoElement {
    view()
        .size(size_pt)
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .accessibility_elements_hidden(true)
        .child(
            svg(ImageSrc::svg_markup(name.markup()))
                .size(size_pt)
                .text_color(color),
        )
}
