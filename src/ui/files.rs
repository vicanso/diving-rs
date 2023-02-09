use bytesize::ByteSize;
use pad::PadStr;
use tui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, List, ListItem, Paragraph},
};

use crate::image::{FileTreeItem, ImageAnalysisResult};

use super::util;

pub struct FilesWidgetOption {
    pub is_active: bool,
    pub selected_layer: usize,
    pub area: Rect,
}

pub struct FilesWidget<'a> {
    // 文件总数
    pub file_count: usize,
    // 组件
    pub files: List<'a>,
    // 文件列表展示区域
    pub files_area: Rect,
    pub block: Block<'a>,
    // block 展示区域
    pub block_area: Rect,
    pub content: Paragraph<'a>,
    // 内容展示区域
    pub content_area: Rect,
}

struct FileTreeView {
    items: Vec<FileTreeItem>,
    width_list: Vec<usize>,
}

impl FileTreeView {
    fn new(items: Vec<FileTreeItem>, width_list: Vec<usize>) -> Self {
        FileTreeView { items, width_list }
    }
    fn add_to_file_tree_view(
        &self,
        list: &mut Vec<ListItem>,
        items: &Vec<FileTreeItem>,
        level: usize,
    ) {
        let space_span = Span::from("   ");
        let permission_width = self.width_list[0];
        let id_width = self.width_list[1];
        let size_width = self.width_list[2];

        let get_file_mode_str = |mode: &str| -> String {
            mode.pad_to_width_with_alignment(permission_width, pad::Alignment::Middle)
        };
        let get_id_str = |id: &str| -> String {
            id.pad_to_width_with_alignment(id_width, pad::Alignment::Right)
        };
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

        let max = items.len();
        for (index, item) in items.iter().enumerate() {
            let style = Style::default();
            let id = format!("{}:{}", item.uid, item.gid);
            let padding = get_padding_str(level, index == max - 1);
            let mut name = item.name.clone();
            if !item.link.is_empty() {
                name = format!("{name} → {}", item.link);
            }
            list.push(ListItem::new(Spans::from(vec![
                Span::styled(get_file_mode_str(&item.mode), style),
                space_span.clone(),
                Span::styled(get_id_str(&id), style),
                space_span.clone(),
                Span::styled(get_size_str(item.size), style),
                space_span.clone(),
                // padding
                Span::from(padding),
                Span::styled(name, style),
            ])));
            if !item.children.is_empty() {
                self.add_to_file_tree_view(list, &item.children, level + 1);
            }
        }
    }
    fn add_to_list(&self, list: &mut Vec<ListItem>) {
        self.add_to_file_tree_view(list, &self.items, 0);
    }
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

    let width_list: Vec<usize> = name_list.iter().map(|item| item.len()).collect();
    let file_tree_items = result.get_layer_file_tree(opt.selected_layer);
    let file_tree_view = FileTreeView::new(file_tree_items, width_list);

    file_tree_view.add_to_list(&mut list);
    let file_count = list.len();
    let files = List::new(list).highlight_style(Style::default().bg(Color::White).fg(Color::Black));
    FilesWidget {
        file_count,
        files,
        files_area: chunks[1],
        block: util::create_block(title),
        block_area: opt.area,
        content,
        content_area: chunks[0],
    }
}
