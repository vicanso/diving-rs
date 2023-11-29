use bytesize::ByteSize;
use pad::PadStr;
use ratatui::{prelude::*, widgets::*};

use super::util;
use crate::image::ImageFileSummary;

pub struct ImageDetailWidget<'a> {
    pub widget: Paragraph<'a>,
}

pub struct ImageDetailWidgetOption {
    pub name: String,
    pub arch: String,
    pub os: String,
    pub total_size: u64,
    pub size: u64,
    pub file_summary_list: Vec<ImageFileSummary>,
}

#[derive(Default, Debug, Clone, PartialEq)]
struct ImageFileWastedSummary {
    pub path: String,
    pub total_size: u64,
    pub count: u32,
}

pub fn new_image_detail_widget<'a>(opt: ImageDetailWidgetOption) -> ImageDetailWidget<'a> {
    let total_size = opt.total_size;
    let size = opt.size;

    let mut wasted_list: Vec<ImageFileWastedSummary> = vec![];
    let mut wasted_size = 0;
    for file in opt.file_summary_list.iter() {
        let mut found = false;
        let info = &file.info;
        wasted_size += info.size;
        for wasted in wasted_list.iter_mut() {
            if wasted.path == info.path {
                found = true;
                wasted.count += 1;
                wasted.total_size += info.size;
            }
        }
        if !found {
            wasted_list.push(ImageFileWastedSummary {
                path: info.path.clone(),
                count: 1,
                total_size: info.size,
            });
        }
    }
    wasted_list.sort_by(|a, b| b.total_size.cmp(&a.total_size));

    let mut score = 100 - wasted_size * 100 / total_size;
    // 有浪费空间，则分数-1
    if wasted_size != 0 {
        score -= 1;
    }

    // 生成浪费空间的文件列表
    let space_span = Span::from("   ");
    let headers = ["Count", "Total Space", "Path"];
    let mut name = opt.name;
    if !opt.arch.is_empty() {
        name += &format!("({}/{})", opt.os, opt.arch);
    }
    let mut spans_list = vec![
        Line::from(vec![
            Span::styled(
                "Image name: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(name),
        ]),
        Line::from(vec![
            Span::styled(
                "Total Image size: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(format!("{} / {}", ByteSize(total_size), ByteSize(size),)),
        ]),
        Line::from(vec![
            Span::styled(
                "Potential wasted space: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(ByteSize(wasted_size).to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "Image efficiency score: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(format!("{score} %")),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled(headers[0], Style::default().add_modifier(Modifier::BOLD)),
            space_span.clone(),
            Span::styled(headers[1], Style::default().add_modifier(Modifier::BOLD)),
            space_span.clone(),
            Span::styled(headers[2], Style::default().add_modifier(Modifier::BOLD)),
        ]),
    ];

    let count_pad_width = headers[0].len();
    let size_pad_width = headers[1].len();

    for wasted in wasted_list.iter() {
        let count_str = format!("{}", wasted.count)
            .pad_to_width_with_alignment(count_pad_width, pad::Alignment::Right);
        let size_str = ByteSize(wasted.total_size)
            .to_string()
            .pad_to_width_with_alignment(size_pad_width, pad::Alignment::Right);
        spans_list.push(Line::from(vec![
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
