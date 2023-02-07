use std::collections::HashMap;

use bytesize::ByteSize;
use pad::PadStr;
use tui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, List, ListItem, Paragraph},
};

use crate::image::{ImageAnalysisResult, CATEGORY_REMOVED};

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
    // 类型：removed, modified
    pub category: Option<String>,
    // 目录树的填充(├ │等)
    pub padding: String,
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
    let get_padding_str = |level: usize, is_last: bool| -> String {
        let mut arr = vec!["│   ".repeat(level)];
        if is_last {
            arr.push("└── ".to_string());
        } else {
            arr.push("├── ".to_string());
        }
        arr.join("")
    };
    // TODO for each生成map，map每次获取数据来调整更新
    if let Some(layer) = result.layers.get(opt.selected_layer) {
        let files = &layer.info.files;

        let mut path_index_map: HashMap<String, usize> = HashMap::with_capacity(files.len() / 10);
        let mut file_tree_items = Vec::with_capacity(files.len());
        for (index, file) in files.iter().enumerate() {
            let arr: Vec<&str> = file.path.split('/').collect();
            let parent_dir = arr[0..arr.len() - 1].join("/");
            for i in 0..arr.len() {
                let mut name = arr[i].to_string();
                // 文件
                if i == arr.len() - 1 {
                    // 如果文件链接至其它文件
                    if !file.link.is_empty() {
                        name = format!("{name} → {}", file.link);
                    }
                    let mut is_last = false;
                    // 如果有下一个文件并且不同路径，是当前目录最后一个文件
                    if let Some(next_file) = layer.info.files.get(index + 1) {
                        if !next_file.path.starts_with(&parent_dir) {
                            is_last = true;
                        }
                    } else {
                        // 如果已无下一下，则是当前目录最后一个文件
                        is_last = true;
                    }
                    // 如果每次计算大小较慢
                    // 后续考虑单独记录
                    file_tree_items.push(FileTreeItem {
                        permission: file.mode.clone(),
                        uid_gid: format!("{}:{}", file.uid, file.gid),
                        size: file.size,
                        name,
                        padding: get_padding_str(arr.len() - 1, is_last),
                        category: result.get_category(&file.path, opt.selected_layer),
                    });
                } else {
                    // 目录的相关处理

                    // 目录完整路径
                    let dir = arr[0..i + 1].join("/");
                    // 如果已存，则目录空间增加
                    if let Some(path_index) = path_index_map.get(&dir) {
                        if let Some(item) = file_tree_items.get_mut(path_index.to_owned()) {
                            item.size += file.size;
                        }
                    } else {
                        path_index_map.insert(dir.clone(), file_tree_items.len());
                        file_tree_items.push(FileTreeItem {
                            uid_gid: "0:0".to_string(),
                            size: file.size,
                            name,
                            padding: get_padding_str(i, false),
                            // name: add_name_padding(&name, i, false),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        let empty_str = &"".to_string();
        for item in file_tree_items.iter() {
            let mut style = Style::default();
            if item.category.as_ref().unwrap_or(empty_str) == CATEGORY_REMOVED {
                style = style.fg(Color::Red);
            }
            list.push(ListItem::new(Spans::from(vec![
                Span::styled(get_file_mode_str(&item.permission), style),
                space_span.clone(),
                Span::styled(get_id_str(&item.uid_gid), style),
                space_span.clone(),
                Span::styled(get_size_str(item.size), style),
                space_span.clone(),
                Span::from(item.padding.clone()),
                Span::styled(item.name.clone(), style),
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
