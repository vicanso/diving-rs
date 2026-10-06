use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use ratatui::{prelude::*, widgets::*};

use super::util;
use crate::i18n;
use crate::image::{DockerAnalyzeSummary, RuntimeCompat};
use crate::recommend::Recommendation;

pub struct ImageDetailWidget<'a> {
    pub widget: Paragraph<'a>,
}

pub struct ImageDetailWidgetOption {
    pub name: String,
    pub arch: String,
    pub os: String,
    pub total_size: u64,
    pub size: u64,
    pub summary: DockerAnalyzeSummary,
    pub recommendations: Vec<Recommendation>,
    pub base_os: String,
    pub runtime_compat: RuntimeCompat,
    /// Set when this is a cached result shown because the registry was
    /// unavailable (see `DockerAnalyzeResult::stale_as_of`).
    pub stale_as_of: Option<String>,
    pub lang: i18n::Lang,
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

pub fn new_image_detail_widget<'a>(opt: ImageDetailWidgetOption) -> ImageDetailWidget<'a> {
    let total_size = opt.total_size;
    let size = opt.size;
    let wasted_size = opt.summary.wasted_size;
    let score = opt.summary.score;
    let wasted_list = opt.summary.wasted_list;

    // let mut wasted_list: Vec<ImageFileWastedSummary> = vec![];
    // let mut wasted_size = 0;
    // for file in opt.file_summary_list.iter() {
    //     let mut found = false;
    //     let info = &file.info;
    //     wasted_size += info.size;
    //     for wasted in wasted_list.iter_mut() {
    //         if wasted.path == info.path {
    //             found = true;
    //             wasted.count += 1;
    //             wasted.total_size += info.size;
    //         }
    //     }
    //     if !found {
    //         wasted_list.push(ImageFileWastedSummary {
    //             path: info.path.clone(),
    //             count: 1,
    //             total_size: info.size,
    //         });
    //     }
    // }
    // wasted_list.sort_by(|a, b| b.total_size.cmp(&a.total_size));

    // let mut score = 100 - wasted_size * 100 / total_size;
    // // 有浪费空间，则分数-1
    // if wasted_size != 0 {
    //     score -= 1;
    // }

    // 生成浪费空间的文件列表
    let space_span = Span::from("   ");
    let headers = [
        i18n::tr(opt.lang, "tui.col.count"),
        i18n::tr(opt.lang, "tui.col.totspace"),
        i18n::tr(opt.lang, "tui.col.path"),
    ];
    let mut name = opt.name;
    if !opt.arch.is_empty() {
        name += &format!("({}/{})", opt.os, opt.arch);
    }
    let mut spans_list = vec![];
    // First thing in the panel, so it stays on screen for as long as the
    // (possibly outdated) numbers below it do.
    if let Some(as_of) = &opt.stale_as_of {
        let warn = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        spans_list.extend(vec![
            Line::from(Span::styled(i18n::tr(opt.lang, "tui.stale"), warn)),
            Line::from(vec![
                Span::styled(i18n::tr(opt.lang, "tui.stale.asof"), warn),
                Span::styled(short_time(as_of), Style::default().fg(Color::Yellow)),
            ]),
            Line::from(vec![]),
        ]);
    }
    spans_list.extend(vec![
        Line::from(vec![
            Span::styled(
                i18n::tr(opt.lang, "tui.imgname"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(name),
        ]),
        Line::from(vec![
            Span::styled(
                i18n::tr(opt.lang, "tui.totsize"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(format!("{} / {}", ByteSize(total_size), ByteSize(size),)),
        ]),
        Line::from(vec![
            Span::styled(
                i18n::tr(opt.lang, "tui.potwasted"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(ByteSize(wasted_size).to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                i18n::tr(opt.lang, "tui.effscore"),
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
                i18n::tr(opt.lang, "tui.baseos"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::from(opt.base_os.clone()),
        ]));
    }
    // Runtime libc (glibc/musl + version requirement vs host) — visible
    // whenever the ELF probe classified the entrypoint successfully. Colored
    // red when the issue tag is non-empty.
    if !opt.runtime_compat.libc.is_empty() {
        let rc = &opt.runtime_compat;
        // Lead with the resolved binary path (incl. "(via wrapper)" when
        // we unwrapped a shell entrypoint) so the user can see which
        // file was actually inspected.
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
            i18n::tr(opt.lang, status_key).to_string()
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
                i18n::tr(opt.lang, "tui.runtimelibc"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(cell, status_style),
        ]));
    }
    spans_list.extend(vec![
        Line::from(vec![]),
        Line::from(vec![
            Span::styled(headers[0], Style::default().add_modifier(Modifier::BOLD)),
            space_span.clone(),
            Span::styled(headers[1], Style::default().add_modifier(Modifier::BOLD)),
            space_span.clone(),
            Span::styled(headers[2], Style::default().add_modifier(Modifier::BOLD)),
        ]),
    ]);

    // Pad data cells to the header's terminal display width (CJK = 2 cols),
    // so the unpadded header row and the padded data rows line up in any
    // language. ASCII headers keep their old width → English is unchanged.
    let count_pad_width = util::get_width(headers[0]) as usize;
    let size_pad_width = util::get_width(headers[1]) as usize;

    for wasted in wasted_list.iter() {
        let count_str = util::pad_display(
            &wasted.count.to_string(),
            count_pad_width,
            util::PadAlign::Right,
        );
        let size_str = util::pad_display(
            &ByteSize(wasted.total_size).to_string(),
            size_pad_width,
            util::PadAlign::Right,
        );
        spans_list.push(Line::from(vec![
            Span::from(count_str),
            space_span.clone(),
            Span::from(size_str),
            space_span.clone(),
            Span::from(format!("/{}", wasted.path)),
        ]))
    }

    if !opt.recommendations.is_empty() {
        spans_list.push(Line::from(vec![]));
        spans_list.push(Line::from(vec![Span::styled(
            i18n::tr(opt.lang, "tui.recs"),
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
                suffix = i18n::fill(
                    i18n::tr(opt.lang, "cli.saved"),
                    &[&ByteSize(r.est_saved_bytes).to_string()],
                );
            }
            if r.heuristic {
                suffix += i18n::tr(opt.lang, "tui.heur");
            }
            spans_list.push(Line::from(vec![
                Span::styled(
                    format!(
                        "[{}] ",
                        i18n::tr(opt.lang, &format!("sev.{}", r.severity)).to_uppercase()
                    ),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::from(format!(
                    "{} — {}{}",
                    i18n::tr(opt.lang, &format!("cat.{}", r.category)),
                    r.title,
                    suffix
                )),
            ]));
        }
    }

    let widget = Paragraph::new(spans_list).block(util::create_block(i18n::tr(
        opt.lang,
        "tui.imgdetails.title",
    )));
    ImageDetailWidget { widget }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    /// Render the panel 44 columns wide — half of a narrow terminal — and
    /// return its rows as text.
    fn render(stale_as_of: Option<&str>, lang: i18n::Lang) -> Vec<String> {
        let widget = new_image_detail_widget(ImageDetailWidgetOption {
            name: "library/redis:alpine".to_string(),
            arch: "amd64".to_string(),
            os: "linux".to_string(),
            total_size: 100,
            size: 50,
            summary: DockerAnalyzeSummary::default(),
            recommendations: vec![],
            base_os: String::new(),
            runtime_compat: RuntimeCompat::default(),
            stale_as_of: stale_as_of.map(str::to_string),
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
        let rows = render(Some("2026-10-06T04:22:25+00:00"), i18n::Lang::En);
        // Row 0 is the border; the notice comes before the image name and
        // fits the narrow panel without being cut off.
        assert!(rows[1].contains("Registry unavailable"), "{rows:#?}");
        assert!(
            rows[2].contains("Cached at: 2026-10-06 04:22 UTC"),
            "{rows:#?}"
        );
        assert!(
            rows[4].contains("Image name: library/redis:alpine"),
            "{rows:#?}"
        );

        let rows = render(None, i18n::Lang::En);
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
