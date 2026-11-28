use ratatui::{prelude::*, widgets::*};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Compute display rows consumed by `lines` when wrapped into `inner_width`
/// columns using word boundaries (matching `ratatui::widgets::Wrap`).
pub fn wrapped_line_count(lines: &[Line<'_>], inner_width: u16, trim: bool) -> u16 {
    let max_width = inner_width.max(1) as usize;
    let mut total = 0usize;
    for line in lines {
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        if text.is_empty() {
            total += 1;
            continue;
        }
        let mut current_width = 0usize;
        let mut line_rows = 1usize;
        let mut chars = text.char_indices().peekable();
        while let Some(&(start, ch)) = chars.peek() {
            let is_ws = ch.is_whitespace();
            while let Some(&(_, c)) = chars.peek() {
                if c.is_whitespace() != is_ws {
                    break;
                }
                chars.next();
            }
            let end = chars.peek().map(|&(idx, _)| idx).unwrap_or(text.len());
            let token = &text[start..end];
            if is_ws {
                if trim && current_width == 0 {
                    continue;
                }
                for c in token.chars() {
                    let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                    if current_width + cw > max_width {
                        line_rows += 1;
                        current_width = 0;
                        if trim {
                            break;
                        }
                    }
                    current_width += cw;
                }
                continue;
            }
            let tok_width = UnicodeWidthStr::width(token);
            if current_width > 0 && current_width + tok_width > max_width {
                line_rows += 1;
                current_width = 0;
            }
            if tok_width <= max_width {
                current_width += tok_width;
            } else {
                for c in token.chars() {
                    let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                    if current_width > 0 && current_width + cw > max_width {
                        line_rows += 1;
                        current_width = 0;
                    }
                    current_width += cw;
                }
            }
        }
        total += line_rows;
    }
    total.min(u16::MAX as usize) as u16
}

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
