use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use ratatui::{prelude::*, widgets::*};

use super::util;
use diving_core::{fill, tr, DockerAnalyzeSummary, Lang, Recommendation, RuntimeCompat};

pub struct ImageDetailWidget<'a> {
    pub widget: Paragraph<'a>,
    pub content_lines: u16,
}

pub struct ImageDetailWidgetOption<'a> {
    pub is_active: bool,
    pub width: u16,
    pub name: &'a str,
    pub arch: &'a str,
    pub os: &'a str,
    pub total_size: u64,
    pub size: u64,
    pub summary: &'a DockerAnalyzeSummary,
    pub recommendations: &'a [Recommendation],
    pub base_os: &'a str,
    pub runtime_compat: &'a RuntimeCompat,
    /// Set when this is a cached result shown because the registry was
    /// unavailable (see `DockerAnalyzeResult::stale_as_of`).
    pub stale_as_of: Option<&'a str>,
    pub lang: Lang,
}

/// `2026-10-06T04:22:25+00:00` → `2026-10-06 04:22 UTC`: this panel is half
/// a terminal wide and does not wrap, so the notice has to stay short.
fn short_time(rfc3339: &str) -> String {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| {
            t.with_timezone(&Utc)
                .format("%Y-%m-%d %H:%M UTC")
                .to_string()
        })
        .unwrap_or_else(|_| rfc3339.to_string())
}

pub fn new_image_detail_widget<'a>(opt: ImageDetailWidgetOption<'a>) -> ImageDetailWidget<'a> {
    let total_size = opt.total_size;
    let size = opt.size;
    let wasted_size = opt.summary.wasted_size;
    let score = opt.summary.score;
    let wasted_list = &opt.summary.wasted_list;

    let space_span = Span::from("   ");
    let headers = [
        tr(opt.lang, "tui.col.count"),
        tr(opt.lang, "tui.col.totspace"),
        tr(opt.lang, "tui.col.path"),
    ];
    let mut name = opt.name.to_string();
    if !opt.arch.is_empty() {
        name += &format!("({}/{})", opt.os, opt.arch);
    }
    let mut spans_list = vec![];
    // First thing in the panel, so it stays on screen for as long as the
    // (possibly outdated) numbers below it do.
    if let Some(as_of) = opt.stale_as_of {
        let warn = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        spans_list.extend(vec![
            Line::from(Span::styled(tr(opt.lang, "tui.stale"), warn)),
            Line::from(vec![
                Span::styled(tr(opt.lang, "tui.stale.asof"), warn),
                Span::styled(short_time(as_of), Style::default().fg(Color::Yellow)),
            ]),
            Line::from(vec![]),
        ]);
    }
    spans_list.extend(vec![
        Line::from(vec![
            Span::styled(
                tr(opt.lang, "tui.imgname"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(name),
        ]),
        Line::from(vec![
            Span::styled(
                tr(opt.lang, "tui.totsize"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(format!("{} / {}", ByteSize(total_size), ByteSize(size),)),
        ]),
        Line::from(vec![
            Span::styled(
                tr(opt.lang, "tui.potwasted"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(ByteSize(wasted_size).to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                tr(opt.lang, "tui.effscore"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(format!("{score} %")),
        ]),
    ]);
    // Base OS — same source as the markdown "Base OS" row. Skip when empty
    // so scratch/distroless images don't show a blank line.
    if !opt.base_os.is_empty() {
        spans_list.push(Line::from(vec![
            Span::styled(
                tr(opt.lang, "tui.baseos"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(opt.base_os),
        ]));
    }
    // Runtime libc (glibc/musl + version requirement vs host) — visible
    // whenever the ELF probe classified the entrypoint successfully. Colored
    // red when the issue tag is non-empty.
    if !opt.runtime_compat.libc.is_empty() {
        let rc = opt.runtime_compat;
        let mut cell = String::new();
        if !rc.entrypoint.is_empty() {
            cell.push_str(&format!("{}: ", rc.entrypoint));
        }
        cell.push_str(&rc.libc);
        if !rc.required_glibc.is_empty() {
            cell.push_str(&format!(" (needs {})", rc.required_glibc));
        }
        if !rc.os_glibc.is_empty() {
            cell.push_str(&format!(" → host {}", rc.os_glibc));
        }
        let status_key = match rc.issue.as_str() {
            "" => "md.runtimelibc.ok",
            "glibc-too-old" => "md.runtimelibc.tooold",
            "glibc-on-musl" => "md.runtimelibc.glibconmusl",
            "musl-on-glibc" => "md.runtimelibc.muslonglibc",
            _ => "",
        };
        let status = if status_key.is_empty() {
            String::new()
        } else {
            tr(opt.lang, status_key).to_string()
        };
        if !status.is_empty() {
            cell.push_str(&format!(" — {status}"));
        }
        let status_style = if rc.issue.is_empty() {
            Style::default()
        } else {
            Style::default().fg(Color::Red)
        };
        spans_list.push(Line::from(vec![
            Span::styled(
                tr(opt.lang, "tui.runtimelibc"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(cell, status_style),
        ]));
    }

    let mut count_pad_width = util::get_width(headers[0]) as usize;
    let mut size_pad_width = util::get_width(headers[1]) as usize;
    let formatted_wasted: Vec<(String, String, &str)> = wasted_list
        .iter()
        .map(|wasted| {
            let c = wasted.count.to_string();
            let s = ByteSize(wasted.total_size).to_string();
            count_pad_width = count_pad_width.max(util::get_width(&c) as usize);
            size_pad_width = size_pad_width.max(util::get_width(&s) as usize);
            (c, s, wasted.path.as_str())
        })
        .collect();

    spans_list.extend(vec![
        Line::from(vec![]),
        Line::from(vec![
            Span::styled(
                util::pad_display(headers[0], count_pad_width, util::PadAlign::Right),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            space_span.clone(),
            Span::styled(
                util::pad_display(headers[1], size_pad_width, util::PadAlign::Right),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            space_span.clone(),
            Span::styled(headers[2], Style::default().add_modifier(Modifier::BOLD)),
        ]),
    ]);

    for (c, s, path) in formatted_wasted {
        let count_str = util::pad_display(&c, count_pad_width, util::PadAlign::Right);
        let size_str = util::pad_display(&s, size_pad_width, util::PadAlign::Right);
        spans_list.push(Line::from(vec![
            Span::from(count_str),
            space_span.clone(),
            Span::from(size_str),
            space_span.clone(),
            Span::from(format!("/{path}")),
        ]))
    }

    if !opt.recommendations.is_empty() {
        spans_list.push(Line::from(vec![]));
        spans_list.push(Line::from(vec![Span::styled(
            tr(opt.lang, "tui.recs"),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        for r in opt.recommendations.iter() {
            let color = match r.severity.as_str() {
                "high" => Color::Red,
                "medium" => Color::Yellow,
                "low" => Color::Green,
                _ => Color::Cyan,
            };
            let mut suffix = String::new();
            if r.est_saved_bytes > 0 {
                suffix = fill(
                    tr(opt.lang, "cli.saved"),
                    &[&ByteSize(r.est_saved_bytes).to_string()],
                );
            }
            if r.heuristic {
                suffix += tr(opt.lang, "tui.heur");
            }
            spans_list.push(Line::from(vec![
                Span::styled(
                    format!(
                        "[{}] ",
                        tr(opt.lang, &format!("sev.{}", r.severity)).to_uppercase()
                    ),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::from(format!(
                    "{} — {}{}",
                    tr(opt.lang, &format!("cat.{}", r.category)),
                    r.title,
                    suffix
                )),
            ]));
        }
    }

    let inner_width = opt.width.saturating_sub(2).max(1);
    let content_lines = util::wrapped_line_count(&spans_list, inner_width, false);
    let title = if opt.is_active {
        tr(opt.lang, "tui.imgdetails.title_active")
    } else {
        tr(opt.lang, "tui.imgdetails.title")
    };
    let widget = Paragraph::new(spans_list)
        .block(util::create_block(title))
        .wrap(Wrap { trim: false });
    ImageDetailWidget {
        widget,
        content_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    /// Render the panel 44 columns wide — half of a narrow terminal — and
    /// return its rows as text.
    fn render(stale_as_of: Option<&str>, lang: Lang) -> Vec<String> {
        let summary = DockerAnalyzeSummary::default();
        let runtime_compat = RuntimeCompat::default();
        let widget = new_image_detail_widget(ImageDetailWidgetOption {
            is_active: false,
            width: 44,
            name: "library/redis:alpine",
            arch: "amd64",
            os: "linux",
            total_size: 100,
            size: 50,
            summary: &summary,
            recommendations: &[],
            base_os: "",
            runtime_compat: &runtime_compat,
            stale_as_of,
            lang,
        });
        let mut terminal = Terminal::new(TestBackend::new(44, 12)).unwrap();
        terminal
            .draw(|f| f.render_widget(widget.widget, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn stale_result_is_flagged_at_the_top_of_the_panel() {
        let rows = render(Some("2026-10-06T04:22:25+00:00"), Lang::En);
        // Row 0 is the border; the notice comes before the image name and
        // fits the narrow panel without being cut off.
        assert!(rows[1].contains("Registry unavailable"), "{rows:#?}");
        assert!(
            rows[2].contains("Cached at: 2026-10-06 04:22 UTC"),
            "{rows:#?}"
        );
        assert!(rows[4].contains("Image name:"), "{rows:#?}");
        assert!(
            format!("{}{}", rows[4], rows[5]).contains("library/redis:alpine(linux/amd64)"),
            "{rows:#?}"
        );

        let rows = render(None, Lang::En);
        assert!(rows[1].contains("Image name: "), "{rows:#?}");
        assert!(!rows.iter().any(|r| r.contains("Registry unavailable")));
    }

    #[test]
    fn short_time_keeps_unparseable_input_as_is() {
        assert_eq!(
            short_time("2026-10-06T12:22:25+08:00"),
            "2026-10-06 04:22 UTC"
        );
        assert_eq!(short_time("yesterday"), "yesterday");
    }
}
