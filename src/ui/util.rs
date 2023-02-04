use tui::{
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders},
};

// 计算字符宽度
pub fn get_width(str: &str) -> u16 {
    // TODO 判断处理宽字符
    let mut count = 0;
    for ch in str.chars() {
        if ch.is_alphabetic() {
            count += 1;
        } else {
            count += 2;
        }
    }

    count
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
