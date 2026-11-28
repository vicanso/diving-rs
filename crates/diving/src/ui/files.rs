use bytesize::ByteSize;
use diving_core::{fill, tr, FileTreeItem, Lang, Op};
use ratatui::{prelude::*, widgets::*};
use std::collections::HashSet;

use super::util;

pub struct FilesWidgetOption<'a> {
    pub is_active: bool,
    pub area: Rect,
    pub mode: u8,
    pub keyword: &'a str,
    pub lang: Lang,
}

pub struct FilesWidget<'a> {
    pub file_count: usize,
    pub files: List<'a>,
    pub files_area: Rect,
    pub block: Block<'a>,
    pub block_area: Rect,
    pub content: Paragraph<'a>,
    pub content_area: Rect,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct FileTreeCacheKey {
    selected_layer: usize,
    mode: u8,
    keyword: String,
    collapsed_version: u64,
}

#[derive(Debug, Clone)]
pub struct CachedFileRow {
    pub full_path: String,
    pub is_dir: bool,
    spans: Vec<(String, Style)>,
}

/// Cached, pre-padded flattened file tree so arrow/page navigation does not
/// re-walk or reformat the layer tree on every keypress.
#[derive(Debug, Default, Clone)]
pub struct FileTreeCache {
    key: Option<FileTreeCacheKey>,
    perm_width: usize,
    id_width: usize,
    size_width: usize,
    pub has_raw_items: bool,
    pub rows: Vec<CachedFileRow>,
}

struct VisibleNode<'a> {
    item: &'a FileTreeItem,
    full_path: String,
    is_dir: bool,
    is_collapsed: bool,
    children: Vec<VisibleNode<'a>>,
}

struct RawRow {
    full_path: String,
    is_dir: bool,
    mode: String,
    uid_gid: String,
    size_str: String,
    tree_prefix: String,
    display_name: String,
    style: Style,
}

fn collect_visible_nodes<'a>(
    items: &'a [FileTreeItem],
    selected_layer: usize,
    mode: u8,
    keyword_lower: &str,
    collapsed: &HashSet<(usize, String)>,
    prefix: &str,
) -> Vec<VisibleNode<'a>> {
    let mut out = Vec::new();
    for item in items {
        let full_path = if prefix.is_empty() {
            item.name.clone()
        } else {
            format!("{prefix}/{}", item.name)
        };
        let is_dir = !item.children.is_empty();
        if is_dir {
            if mode == 2 && item.size < 1024 * 1024 {
                continue;
            }
            let children = collect_visible_nodes(
                &item.children,
                selected_layer,
                mode,
                keyword_lower,
                collapsed,
                &full_path,
            );
            if children.is_empty() {
                continue;
            }
            let is_collapsed = collapsed.contains(&(selected_layer, full_path.clone()));
            out.push(VisibleNode {
                item,
                full_path,
                is_dir: true,
                is_collapsed,
                children,
            });
        } else {
            match mode {
                1 if item.op != Op::Removed && item.op != Op::Modified => continue,
                2 if item.size < 1024 * 1024 => continue,
                _ => {}
            }
            if !keyword_lower.is_empty() && !full_path.to_lowercase().contains(keyword_lower) {
                continue;
            }
            out.push(VisibleNode {
                item,
                full_path,
                is_dir: false,
                is_collapsed: false,
                children: Vec::new(),
            });
        }
    }
    out
}

fn tree_prefix_str(ancestors_last: &[bool], is_last: bool) -> String {
    let mut out = String::with_capacity((ancestors_last.len() + 1) * 4);
    for &last in ancestors_last {
        out.push_str(if last { "    " } else { "│   " });
    }
    out.push_str(if is_last { "└── " } else { "├── " });
    out
}

fn flatten_visible_nodes(
    nodes: &[VisibleNode<'_>],
    ancestors_last: &mut Vec<bool>,
    out: &mut Vec<RawRow>,
) {
    let total = nodes.len();
    for (idx, node) in nodes.iter().enumerate() {
        let is_last = idx + 1 == total;
        let item = node.item;
        let mut style = Style::default();
        match item.op {
            Op::Modified => style = style.fg(Color::Yellow),
            Op::Removed => style = style.fg(Color::Red),
            _ => {}
        }
        let mut display_name = if node.is_dir {
            if node.is_collapsed {
                format!("▸ {}", item.name)
            } else {
                format!("▾ {}", item.name)
            }
        } else {
            item.name.clone()
        };
        if !item.link.is_empty() {
            display_name = format!("{display_name} → {}", item.link);
        }

        out.push(RawRow {
            full_path: node.full_path.clone(),
            is_dir: node.is_dir,
            mode: item.mode.clone(),
            uid_gid: format!("{}:{}", item.uid, item.gid),
            size_str: ByteSize(item.size).to_string(),
            tree_prefix: tree_prefix_str(ancestors_last, is_last),
            display_name,
            style,
        });

        if node.is_dir && !node.is_collapsed && !node.children.is_empty() {
            ancestors_last.push(is_last);
            flatten_visible_nodes(&node.children, ancestors_last, out);
            ancestors_last.pop();
        }
    }
}

pub struct FileTreeCacheQuery<'a> {
    pub file_tree_list: &'a [Vec<FileTreeItem>],
    pub selected_layer: usize,
    pub mode: u8,
    pub keyword: &'a str,
    pub collapsed: &'a HashSet<(usize, String)>,
    pub collapsed_version: u64,
    pub lang: Lang,
}

impl FileTreeCache {
    pub fn ensure_built(&mut self, query: FileTreeCacheQuery<'_>) {
        let FileTreeCacheQuery {
            file_tree_list,
            selected_layer,
            mode,
            keyword,
            collapsed,
            collapsed_version,
            lang,
        } = query;
        let target_key = FileTreeCacheKey {
            selected_layer,
            mode,
            keyword: keyword.to_string(),
            collapsed_version,
        };
        if self.key.as_ref() == Some(&target_key) {
            return;
        }

        let items = file_tree_list
            .get(selected_layer)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        self.has_raw_items = !items.is_empty();

        let keyword_lower = keyword.to_lowercase();
        let visible =
            collect_visible_nodes(items, selected_layer, mode, &keyword_lower, collapsed, "");
        let mut raw_rows = Vec::new();
        let mut ancestors = Vec::new();
        flatten_visible_nodes(&visible, &mut ancestors, &mut raw_rows);

        let mut perm_width = (util::get_width(tr(lang, "tui.col.perm")) as usize).max(10);
        let mut id_width = util::get_width(tr(lang, "tui.col.uidgid")) as usize;
        let mut size_width = util::get_width(tr(lang, "tui.col.fsize")) as usize;
        for r in &raw_rows {
            perm_width = perm_width.max(util::get_width(&r.mode) as usize);
            id_width = id_width.max(util::get_width(&r.uid_gid) as usize);
            size_width = size_width.max(util::get_width(&r.size_str) as usize);
        }

        let gap = ("   ".to_string(), Style::default());
        self.rows = raw_rows
            .into_iter()
            .map(|r| {
                let name_style = if r.is_dir {
                    r.style.add_modifier(Modifier::BOLD)
                } else {
                    r.style
                };
                CachedFileRow {
                    full_path: r.full_path,
                    is_dir: r.is_dir,
                    spans: vec![
                        (
                            util::pad_display(&r.mode, perm_width, util::PadAlign::Middle),
                            r.style,
                        ),
                        gap.clone(),
                        (
                            util::pad_display(&r.uid_gid, id_width, util::PadAlign::Right),
                            r.style,
                        ),
                        gap.clone(),
                        (
                            util::pad_display(&r.size_str, size_width, util::PadAlign::Right),
                            r.style,
                        ),
                        gap.clone(),
                        (r.tree_prefix, Style::default()),
                        (r.display_name, name_style),
                    ],
                }
            })
            .collect();
        self.perm_width = perm_width;
        self.id_width = id_width;
        self.size_width = size_width;
        self.key = Some(target_key);
    }
}

pub fn new_files_widget<'a>(
    cache: &'a FileTreeCache,
    opt: FilesWidgetOption<'a>,
) -> FilesWidget<'a> {
    let title = if opt.is_active {
        tr(opt.lang, "tui.files.title_active")
    } else {
        tr(opt.lang, "tui.files.title")
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(2), Constraint::Length(u16::MAX)].as_ref())
        .split(opt.area);

    let space_span = Span::from("   ");
    let perm_hdr = tr(opt.lang, "tui.col.perm");
    let id_hdr = tr(opt.lang, "tui.col.uidgid");
    let size_hdr = tr(opt.lang, "tui.col.fsize");
    let tree_hdr = tr(opt.lang, "tui.col.filetree");

    let mut mode_tips = fill(tr(opt.lang, "tui.modetips"), &[&opt.mode.to_string()]);
    if !opt.keyword.is_empty() {
        mode_tips.push_str(&format!("  [/{}]", opt.keyword));
    }

    let content = Paragraph::new(vec![
        Line::from(vec![Span::styled(
            mode_tips,
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::from(util::pad_display(
                perm_hdr,
                cache.perm_width,
                util::PadAlign::Left,
            )),
            space_span.clone(),
            Span::from(util::pad_display(
                id_hdr,
                cache.id_width,
                util::PadAlign::Left,
            )),
            space_span.clone(),
            Span::from(util::pad_display(
                size_hdr.trim(),
                cache.size_width,
                util::PadAlign::Right,
            )),
            space_span,
            Span::from(tree_hdr),
        ]),
    ]);

    let file_count = cache.rows.len();
    let list_items: Vec<ListItem<'a>> = if cache.rows.is_empty() {
        let msg_key = if cache.has_raw_items {
            "tui.files.nomatch"
        } else {
            "tui.files.empty"
        };
        vec![ListItem::new(Line::from(Span::styled(
            tr(opt.lang, msg_key),
            Style::default().add_modifier(Modifier::DIM),
        )))]
    } else {
        cache
            .rows
            .iter()
            .map(|row| {
                let spans: Vec<Span<'a>> = row
                    .spans
                    .iter()
                    .map(|(text, style)| Span::styled(text.as_str(), *style))
                    .collect();
                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let files =
        List::new(list_items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn filtered_mode_recomputes_is_last_and_dynamic_column_widths() {
        let tree = vec![vec![
            FileTreeItem {
                name: "etc".into(),
                size: 100,
                children: vec![
                    FileTreeItem {
                        name: "keep.conf".into(),
                        mode: "-rw-r--r--".into(),
                        uid: 65534,
                        gid: 65534,
                        size: 150_000,
                        op: Op::Modified,
                        ..Default::default()
                    },
                    FileTreeItem {
                        name: "unchanged.conf".into(),
                        mode: "-rw-r--r--".into(),
                        uid: 0,
                        gid: 0,
                        size: 50,
                        op: Op::None,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            FileTreeItem {
                name: "var".into(),
                size: 10,
                children: vec![FileTreeItem {
                    name: "other.log".into(),
                    mode: "-rw-r--r--".into(),
                    op: Op::None,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]];
        let mut cache = FileTreeCache::default();
        let collapsed = HashSet::new();
        cache.ensure_built(FileTreeCacheQuery {
            file_tree_list: &tree,
            selected_layer: 0,
            mode: 1,
            keyword: "",
            collapsed: &collapsed,
            collapsed_version: 0,
            lang: Lang::Zh,
        });

        // `var` has no modified/removed files, so `etc` becomes last sibling (`└── `),
        // and `keep.conf` becomes last child (`    └── `).
        assert_eq!(cache.rows.len(), 2);
        assert!(cache.id_width >= "65534:65534".len());
        assert!(cache.size_width >= ByteSize(150_000).to_string().len());
        assert_eq!(cache.rows[0].spans[6].0, "└── ");
        assert_eq!(cache.rows[1].spans[6].0, "    └── ");

        // Empty layer renders cleanly without panic.
        let mut empty_cache = FileTreeCache::default();
        empty_cache.ensure_built(FileTreeCacheQuery {
            file_tree_list: &[],
            selected_layer: 0,
            mode: 0,
            keyword: "",
            collapsed: &collapsed,
            collapsed_version: 0,
            lang: Lang::En,
        });
        assert!(empty_cache.rows.is_empty());
        let widget = new_files_widget(
            &empty_cache,
            FilesWidgetOption {
                is_active: true,
                area: Rect::new(0, 0, 60, 12),
                mode: 0,
                keyword: "",
                lang: Lang::En,
            },
        );
        assert_eq!(widget.file_count, 0);
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let mut list_state = ListState::default();
        term.draw(|f| {
            f.render_widget(widget.block, widget.block_area);
            f.render_widget(widget.content, widget.content_area);
            f.render_stateful_widget(widget.files, widget.files_area, &mut list_state);
        })
        .unwrap();

        // ZH header + row display widths align `大小` cleanly under TestBackend.
        let zh_widget = new_files_widget(
            &cache,
            FilesWidgetOption {
                is_active: true,
                area: Rect::new(0, 0, 70, 10),
                mode: 1,
                keyword: "",
                lang: Lang::Zh,
            },
        );
        let mut zh_term = Terminal::new(TestBackend::new(70, 10)).unwrap();
        let mut zh_state = ListState::default();
        zh_term
            .draw(|f| {
                f.render_widget(zh_widget.block, zh_widget.block_area);
                f.render_widget(zh_widget.content, zh_widget.content_area);
                f.render_stateful_widget(zh_widget.files, zh_widget.files_area, &mut zh_state);
            })
            .unwrap();
        let buf = zh_term.backend().buffer().clone();
        let header_line: String = (0..70).map(|x| buf[(x, 2)].symbol()).collect();
        let child_line: String = (0..70).map(|x| buf[(x, 4)].symbol()).collect();
        // Ratatui fills the second cell of each double-width CJK char with `" "`.
        assert!(header_line.contains("大 小"), "{header_line}");
        assert!(child_line.contains("65534:65534"), "{child_line}");
        let formatted_size = ByteSize(150_000).to_string();
        assert!(child_line.contains(&formatted_size), "{child_line}");
        let hdr_right_col = (0..70u16).find(|&x| buf[(x, 2)].symbol() == "小").unwrap() + 1;
        let size_byte_idx = child_line.find(&formatted_size).unwrap();
        let row_right_col = (child_line[..size_byte_idx].chars().count()
            + formatted_size.chars().count()
            - 1) as u16;
        assert_eq!(hdr_right_col, row_right_col);
    }
}
