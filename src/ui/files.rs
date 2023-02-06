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
struct FileTreeItem {
    pub permission: String,
    pub uid_gid: String,
    pub size: u64,
    pub name: String,
    pub dir: String,
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
    // let mut path_map: HashMap<String, PathInfo> = HashMap::new();

    let get_file_mode_str = |mode: &str| -> String {
        mode.pad_to_width_with_alignment(permission_width, pad::Alignment::Middle)
    };
    let get_id_str =
        |id: &str| -> String { id.pad_to_width_with_alignment(id_width, pad::Alignment::Right) };
    let get_size_str = |size: u64| -> String {
        ByteSize(size)
            .to_string()
            .pad_to_width_with_alignment(size_width, pad::Alignment::Right)
    };
    // TODO 是否last child
    // └──
    let add_name_padding = |name: &str, level: usize| -> String {
        let arr = vec![
            "│  ".repeat(level),
            "├──".to_string(),
            " ".to_string(),
            name.to_string(),
        ];
        arr.join("")
    };
    if let Some(layer) = result.layers.get(opt.selected_layer) {
        let mut file_tree_items = vec![];
        let mut dir_size_map = HashMap::new();
        // 获取index+1与当前对比，判断是否最后一个子目录
        for file in &layer.info.files {
            let arr: Vec<&str> = file.path.split('/').collect();
            for i in 0..arr.len() {
                // 名称
                let mut name = arr[i].to_string();
                // 文件
                if i == arr.len() - 1 {
                    // 如果文件链接至其它文件
                    if !file.link.is_empty() {
                        name = format!("{name} → {}", file.link);
                    }
                    // 文件信息
                    file_tree_items.push(FileTreeItem {
                        permission: file.mode.clone(),
                        uid_gid: format!("{}:{}", file.uid, file.gid),
                        size: file.size,
                        name: add_name_padding(&name, arr.len() - 1),
                        ..Default::default()
                    })
                } else {
                    // 目录完整路径
                    let dir = arr[0..i + 1].join("/");
                    if let Some(size) = dir_size_map.get(&dir) {
                        // 增加该目录下的文件大小
                        dir_size_map.insert(dir, size + file.size);
                    } else {
                        // 如果目录不存在
                        // 记录目录信息
                        file_tree_items.push(FileTreeItem {
                            uid_gid: "0:0".to_string(),
                            size: 0,
                            name: add_name_padding(&name, i),
                            dir: dir.clone(),
                            ..Default::default()
                        });
                        // 记录目录下的文件大小
                        dir_size_map.insert(dir, file.size);
                    }
                }
            }
        }

        for item in file_tree_items.iter() {
            let mut size = item.size;
            if !item.dir.is_empty() {
                size = *dir_size_map.get(&item.dir).unwrap_or(&0);
            }
            list.push(ListItem::new(Spans::from(vec![
                Span::from(get_file_mode_str(&item.permission)),
                space_span.clone(),
                Span::from(get_id_str(&item.uid_gid)),
                space_span.clone(),
                Span::from(get_size_str(size)),
                space_span.clone(),
                Span::from(item.name.clone()),
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
