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
    let total_size = result.total_size;
    let size = result.size;

    let wasted_size = result
        .layer_file_wasted_summary_list
        .iter()
        .map(|item| item.total_size)
        .sum();

    let mut score = 100 - wasted_size * 100 / total_size;
    // 有浪费空间，则分数-1
    if wasted_size != 0 {
        score -= 1;
    }

    // 生成浪费空间的文件列表
    let space_span = Span::from("   ");
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
            Span::from(format!("{} / {}", ByteSize(total_size), ByteSize(size),)),
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
            Span::from(format!("{score} %")),
        ]),
        Spans::from(vec![]),
        Spans::from(vec![
            Span::styled(headers[0], Style::default().add_modifier(Modifier::BOLD)),
            space_span.clone(),
            Span::styled(headers[1], Style::default().add_modifier(Modifier::BOLD)),
            space_span.clone(),
            Span::styled(headers[2], Style::default().add_modifier(Modifier::BOLD)),
        ]),
    ];

    let count_pad_width = headers[0].len();
    let size_pad_width = headers[1].len();

    for wasted in result.layer_file_wasted_summary_list.iter() {
        let count_str = format!("{}", wasted.count)
            .pad_to_width_with_alignment(count_pad_width, pad::Alignment::Right);
        let size_str = ByteSize(wasted.total_size)
            .to_string()
            .pad_to_width_with_alignment(size_pad_width, pad::Alignment::Right);
        spans_list.push(Spans::from(vec![
            Span::from(count_str),
            space_span.clone(),
            Span::from(size_str),
            space_span.clone(),
            Span::from(format!("/{}", wasted.path)),
        ]))
    }

    let widget = Paragraph::new(spans_list).block(util::create_block(" Image Details "));
    ImageDetailWidget { widget }
}
