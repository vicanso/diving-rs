use std::collections::HashMap;

use bytesize::ByteSize;
use chrono::{DateTime, Local, TimeZone};
use pad::PadStr;
use tui::{
    layout::Alignment,
    style::{Modifier, Style},
    text::{Span, Spans},
    widgets::{Paragraph, Wrap},
};

use super::util;
use crate::image::ImageAnalysisResult;

pub struct ImageDetailWidget<'a> {
    pub widget: Paragraph<'a>,
}

pub fn new_image_detail_widget(result: &ImageAnalysisResult) -> ImageDetailWidget {
    let total_size = result.get_image_total_size();
    let size = result.get_image_size();

    let mut wasted_size = 0;
    let mut count_map = HashMap::new();
    let mut size_map = HashMap::new();
    let file_summary_list = result.get_layer_file_summary();
    for item in file_summary_list.iter() {
        wasted_size += item.info.size;
        let key = &item.info.path;
        let size = size_map.get(key).unwrap_or(&0);
        if let Some(value) = count_map.get(key) {
            count_map.insert(key, *value + 1);
        } else {
            count_map.insert(key, 1);
        }
        size_map.insert(key, *size + item.info.size);
    }

    let mut score = 100 - wasted_size * 100 / total_size;
    // 有浪费空间，则分数-1
    if wasted_size != 0 {
        score -= 1;
    }

    // 生成浪费空间的文件列表
    let headers = vec!["Count", "Total Space", "Path"];
    let mut spans_list = vec![
        Spans::from(vec![
            Span::styled(
                "Image name: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(result.name.clone()),
        ]),
        Spans::from(vec![
            Span::styled(
                "Total Image size: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(format!(
                "{} / {}",
                ByteSize(total_size).to_string(),
                ByteSize(size).to_string()
            )),
        ]),
        Spans::from(vec![
            Span::styled(
                "Potential wasted space: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(ByteSize(wasted_size).to_string()),
        ]),
        Spans::from(vec![
            Span::styled(
                "Image efficiency score: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(format!("{} %", score)),
        ]),
        Spans::from(vec![]),
        Spans::from(vec![
            Span::styled(headers[0], Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(headers[1], Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(headers[2], Style::default().add_modifier(Modifier::BOLD)),
        ]),
    ];

    // 减少4个空格
    let size_pad_width = headers[1].len();
    for (key, value) in count_map {
        let size = size_map.get(key).unwrap_or(&0);
        let size_str = ByteSize(*size).to_string();
        spans_list.push(Spans::from(vec![
            Span::from(format!("{}", value)),
            Span::from(size_str.pad_to_width_with_alignment(size_pad_width, pad::Alignment::Right)),
            Span::from(key.clone()),
        ]))
    }

    let widget = Paragraph::new(spans_list).block(util::create_block(" Image Details "));
    ImageDetailWidget { widget }
}
