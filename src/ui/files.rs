use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Corner, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame, Terminal,
};

use crate::image::ImageAnalysisResult;

use super::util;

pub struct FilesWidget<'a> {
    // 组件
    pub widget: List<'a>,
}

pub fn new_files_widget(result: &ImageAnalysisResult) -> FilesWidget {
    let widget =
        List::new(vec![ListItem::new("abc")]).block(util::create_block(" Current Layer Contents "));
    FilesWidget { widget }
}
