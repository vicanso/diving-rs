use chrono::{DateTime, Local, TimeZone};
use ratatui::{prelude::*, widgets::*};

use super::util;
use diving_core::{tr, ImageLayer, Lang};

pub struct DetailWidget<'a> {
    pub height: u16,
    pub widget: Paragraph<'a>,
}

pub struct DetailWidgetOption {
    pub width: u16,
    pub max_height: u16,
    pub lang: Lang,
}

/// Build the layer details widget with `Created:` and `Command:` on separate
/// lines and an accurate wrapped inner height.
pub fn new_layer_detail_widget(layer: &ImageLayer, opt: DetailWidgetOption) -> DetailWidget<'_> {
    let cmd = layer.cmd.clone();
    let mut create_at = layer.created.clone();
    if let Ok(value) = DateTime::parse_from_rfc3339(&layer.created) {
        if let Some(local_dt) = Local.timestamp_opt(value.timestamp(), 0).earliest() {
            create_at = local_dt.to_rfc3339();
        }
    }

    let created_label = format!("{} ", tr(opt.lang, "tui.created"));
    let cmd_label = format!("{} ", tr(opt.lang, "tui.command"));
    let lines = vec![
        Line::from(vec![
            Span::styled(created_label, Style::default().add_modifier(Modifier::BOLD)),
            Span::from(create_at),
        ]),
        Line::from(vec![
            Span::styled(cmd_label, Style::default().add_modifier(Modifier::BOLD)),
            Span::from(cmd),
        ]),
    ];
    let inner_width = opt.width.saturating_sub(2).max(1);
    let content_rows = util::wrapped_line_count(&lines, inner_width, true);
    let mut detail_height = content_rows.saturating_add(2).max(4);
    if opt.max_height > 0 {
        detail_height = detail_height.min(opt.max_height.max(4));
    }
    let paragraph = Paragraph::new(lines)
        .block(util::create_block(tr(opt.lang, "tui.layerdetails.title")))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    DetailWidget {
        height: detail_height,
        widget: paragraph,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

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
                    max_height: 0,
                    lang: Lang::En,
                },
            );
            assert!(widget.height >= 4, "width {width}");
        }
    }

    #[test]
    fn created_and_command_render_on_separate_lines() {
        let layer = ImageLayer {
            created: "2024-06-01T00:00:00Z".to_string(),
            cmd: "RUN echo hello world".to_string(),
            ..Default::default()
        };
        let detail = new_layer_detail_widget(
            &layer,
            DetailWidgetOption {
                width: 60,
                max_height: 12,
                lang: Lang::En,
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(60, detail.height)).unwrap();
        terminal
            .draw(|f| f.render_widget(detail.widget, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        assert!(rows[1].contains("Created: "), "{rows:#?}");
        assert!(!rows[1].contains("Command:"), "{rows:#?}");
        assert!(
            rows[2].contains("Command: RUN echo hello world"),
            "{rows:#?}"
        );
    }
}
