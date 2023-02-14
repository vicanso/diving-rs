use bytesize::ByteSize;
use pad::PadStr;
use tui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, List, ListItem, Paragraph},
};

use crate::image::{FileTreeItem, Op};

use super::util;

pub struct FilesWidgetOption {
    pub is_active: bool,
    pub selected_layer: usize,
    pub area: Rect,
    pub mode: u8,
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

fn add_to_file_tree_view(
    mode: u8,
    width_list: Vec<usize>,
    list: &mut Vec<ListItem>,
    items: &Vec<FileTreeItem>,
    is_last_list: Vec<bool>,
) -> usize {
    let mut count = 0;
    let space_span = Span::from("   ");
    let permission_width = width_list[0];
    let id_width = width_list[1];
    let size_width = width_list[2];

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
    let get_padding_str = |list: &[bool], is_last: bool| -> String {
        let mut arr: Vec<String> = list
            .iter()
            .map(|is_last| if is_last.to_owned() { "    " } else { "│   " }.to_string())
            .collect();
        if is_last {
            arr.push("└── ".to_string());
        } else {
            arr.push("├── ".to_string());
        }
        arr.join("")
    };

    let max = items.len();
    for (index, item) in items.iter().enumerate() {
        match mode {
            // 只展示更新与删除
            1 => {
                if item.op != Op::Remove && item.op != Op::Modified {
                    continue;
                }
            }
            // 只显示大于1MB
            2 => {
                if item.size < 1024 * 1024 {
                    continue;
                }
            }
            _ => {}
        }
        let mut style = Style::default();
        match item.op {
            Op::Modified => style = style.fg(Color::Yellow),
            Op::Remove => style = style.fg(Color::Red),
            _ => {}
        }
        let id = format!("{}:{}", item.uid, item.gid);
        let is_last = index == max - 1;
        let padding = get_padding_str(&is_last_list, is_last);
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
        count += 1;
        if !item.children.is_empty() {
            let mut tmp = is_last_list.clone();
            tmp.push(is_last);

            // 如果子元素没有符合插入到列表的
            // 则当前元素也删除
            let child_append_count =
                add_to_file_tree_view(mode, width_list.clone(), list, &item.children, tmp);
            if child_append_count == 0 {
                list.pop();
                count -= 1;
            }
        }
    }
    count
}

pub fn new_files_widget(
    file_tree_list: &[Vec<FileTreeItem>],
    opt: FilesWidgetOption,
) -> FilesWidget {
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
    let name_list = vec!["Permission", " UID:GID ", "    Size", "FileTree"];
    let content = Paragraph::new(vec![
        Spans::from(vec![Span::styled(
            "0:All 1:Modified/Removed 2:File >= 1MB",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
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
    let file_tree_items = &file_tree_list[opt.selected_layer];

    add_to_file_tree_view(opt.mode, width_list, &mut list, file_tree_items, vec![]);

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
