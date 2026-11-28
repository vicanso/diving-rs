use super::util;
use bytesize::ByteSize;
use diving_core::{tr, ImageLayer, Lang};
use ratatui::{prelude::*, widgets::*};

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerOrder {
    #[default]
    Desc,
    Asc,
}

impl LayerOrder {
    pub fn toggle(self) -> Self {
        match self {
            Self::Desc => Self::Asc,
            Self::Asc => Self::Desc,
        }
    }
}

pub struct LayersWidget<'a> {
    pub height: u16,
    pub widget: Table<'a>,
}

pub struct LayersWidgetOption {
    pub is_active: bool,
    pub lang: Lang,
    pub max_height: u16,
    pub order: LayerOrder,
}

/// Build the scrollable layers table widget, bounded to `opt.max_height` so
/// lower panels remain visible on small terminals.
pub fn new_layers_widget<'a>(layers: &[ImageLayer], opt: LayersWidgetOption) -> LayersWidget<'a> {
    let idx_hdr = format!(
        "{} {}",
        tr(opt.lang, "tui.col.index"),
        if opt.order == LayerOrder::Desc {
            "↓"
        } else {
            "↑"
        }
    );
    let size_hdr = tr(opt.lang, "tui.col.size");
    let cmd_hdr = tr(opt.lang, "tui.col.command");

    let mut col_widths = [
        (util::get_width(&idx_hdr) as usize).max(5),
        (util::get_width(size_hdr) as usize).max(10),
    ];
    let mut row_data_list = Vec::with_capacity(layers.len());
    let ordered_indices: Box<dyn Iterator<Item = usize>> = match opt.order {
        LayerOrder::Desc => Box::new((0..layers.len()).rev()),
        LayerOrder::Asc => Box::new(0..layers.len()),
    };
    for index in ordered_indices {
        let item = &layers[index];
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
            &idx_hdr,
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

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    fn render_table(layers: &[ImageLayer], order: LayerOrder) -> String {
        let w = new_layers_widget(
            layers,
            LayersWidgetOption {
                is_active: true,
                lang: Lang::En,
                max_height: 10,
                order,
            },
        );
        let mut state = TableState::default();
        state.select(Some(0));
        let mut term = Terminal::new(TestBackend::new(60, w.height)).unwrap();
        term.draw(|f| f.render_stateful_widget(w.widget, f.area(), &mut state))
            .unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn layer_table_renders_reverse_by_default_and_toggles_to_ascending() {
        let layers = vec![
            ImageLayer {
                cmd: "FROM alpine".into(),
                size: 100,
                ..Default::default()
            },
            ImageLayer {
                cmd: "RUN app".into(),
                size: 200,
                ..Default::default()
            },
        ];
        let desc = render_table(&layers, LayerOrder::default());
        assert!(desc.contains("Index ↓"), "{desc}");
        let pos2 = desc.find("RUN app").unwrap();
        let pos1 = desc.find("FROM alpine").unwrap();
        assert!(pos2 < pos1, "expected layer 2 above layer 1:\n{desc}");

        let asc = render_table(&layers, LayerOrder::Asc);
        assert!(asc.contains("Index ↑"), "{asc}");
        let pos2 = asc.find("RUN app").unwrap();
        let pos1 = asc.find("FROM alpine").unwrap();
        assert!(pos1 < pos2, "expected layer 1 above layer 2:\n{asc}");
    }
}
