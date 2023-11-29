use chrono::{DateTime, Local, TimeZone};
use ratatui::{prelude::*, widgets::*};

use super::util;
use crate::image::ImageLayer;

pub struct DetailWidget<'a> {
    // 组件高度
    pub height: u16,
    // 组件
    pub widget: Paragraph<'a>,
}
pub struct DetailWidgetOption {
    pub width: u16,
}
// 创建layer详细信息的widget
pub fn new_layer_detail_widget(layer: &ImageLayer, opt: DetailWidgetOption) -> DetailWidget {
    let cmd = layer.cmd.clone();
    let detail_word_width = util::get_width(&cmd);
    let mut create_at = layer.created.clone();
    if let Ok(value) = DateTime::parse_from_rfc3339(&layer.created) {
        create_at = Local
            .timestamp_opt(value.timestamp(), 0)
            .single()
            .unwrap()
            .to_rfc3339();
    };

    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled("Created:", Style::default().add_modifier(Modifier::BOLD)),
        Span::from(create_at),
        Span::styled("Command:", Style::default().add_modifier(Modifier::BOLD)),
        Span::from(cmd),
    ]))
    .block(util::create_block(" Layer Details "))
    .alignment(Alignment::Left)
    .wrap(Wrap { trim: true });
    // 拆分左侧栏
    let mut detail_height = detail_word_width / opt.width;
    if detail_word_width % opt.width != 0 {
        detail_height += 1;
    }
    // title + command tag + created tag + created time + border bottom
    detail_height += 5;
    DetailWidget {
        height: detail_height,
        widget: paragraph,
    }
}
