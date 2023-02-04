use tui::{
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders},
};
use unicode_width::UnicodeWidthStr;

// 计算字符宽度
pub fn get_width(str: &str) -> u16 {
    UnicodeWidthStr::width(str) as u16
}

// 创建block
pub fn create_block(title: &str) -> Block {
    Block::default().borders(Borders::ALL).title(Span::styled(
        title,
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

// 标题设置为活动状态
pub fn wrap_active(title: &str) -> String {
    format!(" ●{title}")
}
