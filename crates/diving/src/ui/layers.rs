use super::util;
use bytesize::ByteSize;
use diving_core::{tr, ImageLayer, Lang};
use ratatui::{prelude::*, widgets::*};

pub struct LayersWidget<'a> {
    pub height: u16,
    pub widget: Table<'a>,
}

pub struct LayersWidgetOption {
    pub is_active: bool,
    pub lang: Lang,
    pub max_height: u16,
}

/// Build the scrollable layers table widget, bounded to `opt.max_height` so
/// lower panels remain visible on small terminals.
pub fn new_layers_widget<'a>(layers: &[ImageLayer], opt: LayersWidgetOption) -> LayersWidget<'a> {
    let idx_hdr = tr(opt.lang, "tui.col.index");
    let size_hdr = tr(opt.lang, "tui.col.size");
    let cmd_hdr = tr(opt.lang, "tui.col.command");

    let mut col_widths = [
        (util::get_width(idx_hdr) as usize).max(5),
        (util::get_width(size_hdr) as usize).max(10),
    ];
    let mut row_data_list = Vec::with_capacity(layers.len());
    for (index, item) in layers.iter().enumerate() {
        let no = format!("{}", index + 1);
        let sz = ByteSize(item.size).to_string();
        col_widths[0] = col_widths[0].max(util::get_width(&no) as usize);
        col_widths[1] = col_widths[1].max(util::get_width(&sz) as usize);
        row_data_list.push((no, sz, item.cmd.clone()));
    }

    let rows: Vec<Row<'a>> = if row_data_list.is_empty() {
        vec![Row::new(vec![
            Cell::from(""),
            Cell::from(""),
            Cell::from(tr(opt.lang, "tui.layers.empty"))
                .style(Style::default().add_modifier(Modifier::DIM)),
        ])
        .height(1)]
    } else {
        row_data_list
            .into_iter()
            .map(|(no, sz, cmd)| {
                Row::new(vec![
                    Cell::from(util::pad_display(&no, col_widths[0], util::PadAlign::Right)),
                    Cell::from(util::pad_display(&sz, col_widths[1], util::PadAlign::Right)),
                    Cell::from(cmd),
                ])
                .height(1)
            })
            .collect()
    };

    let natural_height = (3 + rows.len()) as u16;
    let height = natural_height.min(opt.max_height.max(4));
    let header = Row::new([
        Cell::from(util::pad_display(
            idx_hdr,
            col_widths[0],
            util::PadAlign::Right,
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(util::pad_display(
            size_hdr,
            col_widths[1],
            util::PadAlign::Right,
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(cmd_hdr).style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .height(1);

    let title = if opt.is_active {
        tr(opt.lang, "tui.layers.title_active")
    } else {
        tr(opt.lang, "tui.layers.title")
    };
    let widget = Table::new(
        rows,
        [
            Constraint::Length(col_widths[0] as u16),
            Constraint::Length(col_widths[1] as u16),
            Constraint::Fill(1),
        ],
    )
    .header(header)
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
    .block(util::create_block(title));

    LayersWidget { height, widget }
}
