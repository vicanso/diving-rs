use bytesize::ByteSize;

use pad::PadStr;

use tui::{
    layout::Constraint,
    style::{Color, Modifier, Style},
    text::Spans,
    widgets::{Cell, Row, Table},
};

use super::util;
use crate::image::ImageLayer;

pub struct LayersWidget<'a> {
    // 组件高度
    pub height: u16,
    // 组件
    pub widget: Table<'a>,
}
pub struct LayersWidgetOption {
    pub is_active: bool,
    pub selected_layer: usize,
}
// 创建layer列表的widget
pub fn new_layers_widget<'a>(layers: &[ImageLayer], opt: LayersWidgetOption) -> LayersWidget<'a> {
    let mut row_max_counts = [0, 0, 0];
    let mut row_data_list = vec![];
    // 生成表格数据，并计算每列最大宽度
    for (index, item) in layers.iter().enumerate() {
        let no = format!("{}", index + 1);
        // TODO 是否调整为1024
        let arr = vec![no, ByteSize(item.size).to_string(), item.cmd.clone()];
        for (i, value) in arr.iter().enumerate() {
            if row_max_counts[i] < value.len() {
                row_max_counts[i] = value.len()
            }
        }
        row_data_list.push(arr)
    }

    let mut rows = vec![];
    for (index, arr) in row_data_list.into_iter().enumerate() {
        let mut cells = vec![];
        // 前两列填充空格
        for (i, value) in arr.into_iter().enumerate() {
            if i != 2 {
                cells.push(Cell::from(value.pad_to_width_with_alignment(
                    row_max_counts[i],
                    pad::Alignment::Right,
                )));
            } else {
                cells.push(Cell::from(Spans::from(value)));
            }
        }
        let mut style = Style::default();
        if index == opt.selected_layer {
            style = style.bg(Color::White).fg(Color::Black);
        }

        rows.push(Row::new(cells).style(style).height(1))
    }

    let headers = ["Index", "Size", "Command"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));
    // title + header + border bottom
    let height = 3 + rows.len();
    let header = Row::new(headers).height(1);
    // TODO 如何调整生命周期
    let mut title = " Layers ";
    if opt.is_active {
        title = " ● Layers ";
    }
    let widget = Table::new(rows)
        .header(header)
        .block(util::create_block(title))
        .widths(&[
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Min(u16::MAX),
        ]);

    LayersWidget {
        height: height as u16,
        widget,
    }
}
