use ratatui::{prelude::*, widgets::*};

use unicode_width::UnicodeWidthStr;

// 计算字符宽度
pub fn get_width(str: &str) -> u16 {
    UnicodeWidthStr::width_cjk(str) as u16
}

// 创建block
pub fn create_block(title: &str) -> Block {
    Block::default().borders(Borders::ALL).title(Span::styled(
        title,
        Style::default().add_modifier(Modifier::BOLD),
    ))
}
