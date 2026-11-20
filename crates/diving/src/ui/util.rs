use ratatui::{prelude::*, widgets::*};

use unicode_width::UnicodeWidthStr;

// 计算字符宽度
pub fn get_width(str: &str) -> u16 {
    UnicodeWidthStr::width_cjk(str) as u16
}

// 创建block
pub fn create_block(title: &str) -> Block<'_> {
    Block::default().borders(Borders::ALL).title(Span::styled(
        title,
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

#[derive(Clone, Copy)]
pub enum PadAlign {
    Left,
    Right,
    Middle,
}

// 按终端显示宽度（CJK 字符占 2 列）将字符串补足到 width；
// 已达到/超过 width 时原样返回（与原 pad 行为一致，不截断）。
pub fn pad_display(s: &str, width: usize, align: PadAlign) -> String {
    let w = UnicodeWidthStr::width_cjk(s);
    if w >= width {
        return s.to_string();
    }
    let pad = width - w;
    match align {
        PadAlign::Left => format!("{s}{}", " ".repeat(pad)),
        PadAlign::Right => format!("{}{s}", " ".repeat(pad)),
        PadAlign::Middle => {
            let left = pad / 2;
            format!("{}{s}{}", " ".repeat(left), " ".repeat(pad - left))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_display_ascii_matches_byte_width() {
        assert_eq!(pad_display("abc", 5, PadAlign::Left), "abc  ");
        assert_eq!(pad_display("abc", 5, PadAlign::Right), "  abc");
        // Already wide enough → unchanged, never truncated.
        assert_eq!(pad_display("abcdef", 4, PadAlign::Left), "abcdef");
    }

    #[test]
    fn pad_display_counts_cjk_as_two_columns() {
        // "权限" is 2 chars but 4 display columns → pad to 6 adds 2 spaces.
        assert_eq!(pad_display("权限", 6, PadAlign::Left), "权限  ");
        assert_eq!(UnicodeWidthStr::width_cjk("权限  "), 6);
    }
}
