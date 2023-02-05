use std::collections::HashMap;

use bytesize::ByteSize;
use pad::PadStr;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Corner, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};

use crate::image::ImageAnalysisResult;

use super::util;

pub struct FilesWidgetOption {
    pub is_active: bool,
    pub selected_layer: usize,
    pub area: Rect,
}

pub struct FilesWidget<'a> {
    // 组件
    pub files: List<'a>,
    pub files_area: Rect,
    pub block: Block<'a>,
    pub block_area: Rect,
    pub content: Paragraph<'a>,
    pub content_area: Rect,
}

#[derive(Default, Debug, Clone)]
struct PathInfo {
    size: u64,
}

pub fn new_files_widget(result: &ImageAnalysisResult, opt: FilesWidgetOption) -> FilesWidget {
    // TODO 如何调整生命周期
    let mut title = " Current Layer Contents ";
    if opt.is_active {
        title = " ● Current Layer Contents ";
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(2), Constraint::Length(u16::MAX)].as_ref())
        .split(opt.area);

    let space_span = Span::from("   ");
    let name_list = vec!["Permission", "UID:GID", "    Size", "FileTree"];
    let content = Paragraph::new(vec![
        Spans::from(vec![Span::from("")]),
        Spans::from(vec![
            Span::from(name_list[0]),
            space_span.clone(),
            Span::from(name_list[1]),
            space_span.clone(),
            Span::from(name_list[2]),
            space_span.clone(),
            Span::from(name_list[3]),
        ]),
    ]);

    let mut list = vec![];
    let permission_width = name_list[0].len();
    let id_width = name_list[1].len();
    let size_width = name_list[2].len();
    let mut path_map: HashMap<String, PathInfo> = HashMap::new();
    let get_dirs = |path: &str| -> Vec<String> {
        let arr: Vec<&str> = path.split('/').collect();
        let mut dirs = vec![];
        for i in 0..arr.len() - 1 {
            dirs.push(arr[0..i + 1].join("/"));
        }
        dirs
    };
    let get_file_mode_str = |mode: &str| -> String {
        mode.pad_to_width_with_alignment(permission_width, pad::Alignment::Middle)
    };
    let get_id_str = |uid: u64, gid: u64| -> String {
        format!("{uid}:{gid}").pad_to_width_with_alignment(id_width, pad::Alignment::Right)
    };
    let get_size_str = |size: u64| -> String {
        ByteSize(size)
            .to_string()
            .pad_to_width_with_alignment(size_width, pad::Alignment::Right)
    };
    if let Some(layer) = result.layers.get(opt.selected_layer) {
        for file in &layer.info.files {
            for dir in get_dirs(&file.path).iter() {
                if let Some(info) = path_map.get_mut(dir) {
                    info.size += file.size;
                } else {
                    path_map.insert(dir.clone(), PathInfo { size: file.size });
                }
            }
        }
        for (index, file) in layer.info.files.iter().enumerate() {
            let dirs = get_dirs(&file.path);
            for dir in dirs.iter() {
                if let Some(info) = path_map.get_mut(dir) {
                    list.push(ListItem::new(Spans::from(vec![
                        Span::from(get_file_mode_str("-")),
                        space_span.clone(),
                        Span::from(get_id_str(0, 0)),
                        space_span.clone(),
                        Span::from(get_size_str(info.size)),
                        space_span.clone(),
                        Span::from(dir.clone()),
                    ])));
                    path_map.remove(dir);
                }
            }
            list.push(ListItem::new(Spans::from(vec![
                Span::from(get_file_mode_str(&file.mode)),
                space_span.clone(),
                Span::from(get_id_str(file.uid, file.gid)),
                space_span.clone(),
                Span::from(get_size_str(file.size)),
                space_span.clone(),
                Span::from(file.path.clone()),
            ])));
        }
    }
    let files = List::new(list).highlight_style(Style::default().bg(Color::White).fg(Color::Black));
    FilesWidget {
        files,
        files_area: chunks[1],
        block: util::create_block(title),
        block_area: opt.area,
        content,
        content_area: chunks[0],
    }
}
