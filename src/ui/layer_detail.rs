use chrono::{DateTime, Local, TimeZone};
use ratatui::{prelude::*, widgets::*};

use super::util;
use crate::i18n;
use crate::image::ImageLayer;

pub struct DetailWidget<'a> {
    // 组件高度
    pub height: u16,
    // 组件
    pub widget: Paragraph<'a>,
}
pub struct DetailWidgetOption {
    pub width: u16,
    pub lang: i18n::Lang,
}
// 创建layer详细信息的widget
pub fn new_layer_detail_widget(layer: &ImageLayer, opt: DetailWidgetOption) -> DetailWidget<'_> {
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
        Span::styled(
            i18n::tr(opt.lang, "tui.created"),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::from(create_at),
        Span::styled(
            i18n::tr(opt.lang, "tui.command"),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::from(cmd),
    ]))
    .block(util::create_block(i18n::tr(
        opt.lang,
        "tui.layerdetails.title",
    )))
    .alignment(Alignment::Left)
    .wrap(Wrap { trim: true });
    // 拆分左侧栏。没有尺寸的伪终端会报告宽度 0（管道输入的 `script`、
    // 某些容器环境），按 1 处理，避免除以零直接崩溃。
    let width = opt.width.max(1);
    let mut detail_height = detail_word_width / width;
    if detail_word_width.is_multiple_of(width) {
        detail_height += 1;
    }
    // title + command tag + created tag + created time + border bottom
    detail_height += 5;
    DetailWidget {
        height: detail_height,
        widget: paragraph,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_width_terminal_does_not_panic() {
        let layer = ImageLayer {
            cmd: "RUN apt-get update && apt-get install -y curl".to_string(),
            ..Default::default()
        };
        for width in [0, 1, 40] {
            let widget = new_layer_detail_widget(
                &layer,
                DetailWidgetOption {
                    width,
                    lang: i18n::Lang::En,
                },
            );
            assert!(widget.height >= 5, "width {width}");
        }
    }
}
