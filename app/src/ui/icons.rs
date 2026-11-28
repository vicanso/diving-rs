//! Single-family SVG icons rendered natively via `nsh_core::prelude::svg`.

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

use crate::domain::FileKind;
use crate::ui::theme::file_kind_color;

const INFO_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 16v-4"/><path d="M12 8h.01"/></svg>"#;
const LOADER_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-6.219-8.56"/></svg>"#;

/// Identifiers for every UI icon bundled in `app/assets/icons/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconName {
    Logo,
    Search,
    Folder,
    FolderOpen,
    File,
    FileImage,
    FileAudio,
    FileVideo,
    FileCode,
    FileConfig,
    FileText,
    FileArchive,
    FileTerminal,
    FileBinary,
    FileKey,
    FileDatabase,
    FileFont,
    Link,
    Layers,
    Treemap,
    Findings,
    Info,
    Loader,
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
    PanelLeft,
    Plus,
    SortOrder,
    Trash,
}

impl IconName {
    fn markup(self) -> &'static str {
        match self {
            Self::Logo => include_str!("../../assets/icons/logo.svg"),
            Self::Search => include_str!("../../assets/icons/search.svg"),
            Self::Folder => include_str!("../../assets/icons/folder.svg"),
            Self::FolderOpen => include_str!("../../assets/icons/folder-open.svg"),
            Self::File => include_str!("../../assets/icons/file.svg"),
            Self::FileImage => include_str!("../../assets/icons/file-image.svg"),
            Self::FileAudio => include_str!("../../assets/icons/file-audio.svg"),
            Self::FileVideo => include_str!("../../assets/icons/file-video.svg"),
            Self::FileCode => include_str!("../../assets/icons/file-code.svg"),
            Self::FileConfig => include_str!("../../assets/icons/file-config.svg"),
            Self::FileText => include_str!("../../assets/icons/file-text.svg"),
            Self::FileArchive => include_str!("../../assets/icons/file-archive.svg"),
            Self::FileTerminal => include_str!("../../assets/icons/file-terminal.svg"),
            Self::FileBinary => include_str!("../../assets/icons/file-binary.svg"),
            Self::FileKey => include_str!("../../assets/icons/file-key.svg"),
            Self::FileDatabase => include_str!("../../assets/icons/file-database.svg"),
            Self::FileFont => include_str!("../../assets/icons/file-font.svg"),
            Self::Link => include_str!("../../assets/icons/link.svg"),
            Self::Layers => include_str!("../../assets/icons/layers.svg"),
            Self::Treemap => include_str!("../../assets/icons/treemap.svg"),
            Self::Findings => include_str!("../../assets/icons/findings.svg"),
            Self::Info => INFO_SVG,
            Self::Loader => LOADER_SVG,
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
            Self::PanelLeft => include_str!("../../assets/icons/panel-left.svg"),
            Self::Plus => include_str!("../../assets/icons/plus.svg"),
            Self::SortOrder => include_str!("../../assets/icons/sort-order.svg"),
            Self::Trash => include_str!("../../assets/icons/trash.svg"),
        }
    }
}

/// Maps a semantic [`FileKind`] to its single-family [`IconName`].
pub fn file_kind_icon(kind: FileKind) -> IconName {
    match kind {
        FileKind::Symlink => IconName::Link,
        FileKind::Image => IconName::FileImage,
        FileKind::Audio => IconName::FileAudio,
        FileKind::Video => IconName::FileVideo,
        FileKind::Font => IconName::FileFont,
        FileKind::Code => IconName::FileCode,
        FileKind::Config => IconName::FileConfig,
        FileKind::Text => IconName::FileText,
        FileKind::Archive => IconName::FileArchive,
        FileKind::Script => IconName::FileTerminal,
        FileKind::Binary => IconName::FileBinary,
        FileKind::Key => IconName::FileKey,
        FileKind::Database => IconName::FileDatabase,
        FileKind::Dockerfile => IconName::Dockerfile,
        FileKind::Generic => IconName::File,
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

/// Renders a file-type icon tinted by its theme-adaptive semantic [`FileKind`] token.
pub fn ui_file_icon(kind: FileKind, size_pt: Points) -> impl IntoElement {
    ui_icon(file_kind_icon(kind), size_pt, move || file_kind_color(kind))
}

/// Renders a linear-stroke loading spinner rotating smoothly on the UI thread (static under reduced motion).
pub fn ui_spinner(size_pt: Points, color: impl IntoReactive<AdaptiveColor>) -> impl IntoElement {
    let angle = animated_value(0.0);
    if !reduce_motion() {
        let _ = Animation::timing(&angle, 360.0, Timing::new(900.0, Easing::Linear))
            .looped(None)
            .start();
    }
    view()
        .size(size_pt)
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .accessibility_elements_hidden(true)
        .bind_rotate(&angle)
        .child(
            svg(ImageSrc::svg_markup(IconName::Loader.markup()))
                .size(size_pt)
                .text_color(color),
        )
}
