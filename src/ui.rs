use bytesize::ByteSize;
use chrono::{DateTime, Local, TimeZone, Utc};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pad::PadStr;
use std::{cell, error::Error, io, thread, time::Duration, vec};
use tracing_subscriber::layer;
use tui::{
    backend::Backend,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Widget, Wrap},
    Frame, Terminal,
};

use crate::image::{ImageAnalysisResult, ImageLayer};

fn get_width(str: &str) -> u16 {
    // TODO 判断处理宽字符
    let mut count = 0;
    for ch in str.chars() {
        if ch.is_alphabetic() {
            count += 1;
        } else {
            count += 2;
        }
    }

    count
}

fn create_block(title: &str) -> Block {
    Block::default().borders(Borders::ALL).title(Span::styled(
        title,
        Style::default().add_modifier(Modifier::BOLD),
    ))
}
fn wrap_active(title: &str) -> String {
    format!(" ●{}", title)
}

#[derive(Default, Debug, Clone)]
struct WidgetState {
    // 选中的区域
    active: u8,
}

impl WidgetState {
    fn next_widget(&mut self) {
        self.active += 1;
    }
}

pub fn run_app(result: ImageAnalysisResult) -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let mut state = WidgetState {
        ..Default::default()
    };
    loop {
        terminal.draw(|f| draw_widgets(f, &result, &state))?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                // 退出
                KeyCode::Char('c') => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        break;
                    }
                }
                // 退出
                KeyCode::Char('q') => break,
                KeyCode::Tab => state.next_widget(),
                _ => continue,
            }
        }
    }

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

struct DetailWidget<'a> {
    // 组件高度
    height: u16,
    // 组件
    widget: Paragraph<'a>,
}
// 创建layer详细信息的widget
fn new_layer_detail_widget(layer: &ImageLayer, width: u16) -> DetailWidget {
    let cmd = layer.cmd.clone();
    let detail_word_width = get_width(&cmd);
    let mut create_at = layer.created.clone();
    if let Ok(value) = DateTime::parse_from_rfc3339(&layer.created) {
        create_at = Local
            .timestamp_opt(value.timestamp(), 0)
            .single()
            .unwrap()
            .to_rfc3339();
    };

    let paragraph = Paragraph::new(vec![
        Spans::from(Span::styled(
            "Command:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Spans::from(cmd),
        Spans::from(Span::styled(
            "Created:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Spans::from(create_at),
    ])
    .block(create_block(" Layer Details "))
    .alignment(Alignment::Left)
    .wrap(Wrap { trim: true });
    // 拆分左侧栏
    let mut detail_height = detail_word_width / width;
    if detail_word_width % width != 0 {
        detail_height += 1;
    }
    // title + command tag + created tag + created time + border bottom
    detail_height += 5;
    DetailWidget {
        height: detail_height,
        widget: paragraph,
    }
}

fn draw_widgets<B: Backend>(f: &mut Frame<B>, result: &ImageAnalysisResult, state: &WidgetState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());

    let mut row_max_counts = vec![0, 0, 0];
    let mut row_data_list = vec![];
    // 生成表格数据，并计算每列最大宽度
    for (index, item) in result.layers.iter().enumerate() {
        let no = format!("{index}");
        let arr = vec![no, ByteSize(item.size).to_string(), item.cmd.clone()];
        for (i, value) in arr.iter().enumerate() {
            if row_max_counts[i] < value.len() {
                row_max_counts[i] = value.len()
            }
        }
        row_data_list.push(arr)
    }

    let rows = row_data_list.iter().map(|arr| {
        let mut cells = vec![];
        // 前两列填充空格
        for (i, value) in arr.iter().enumerate() {
            if i != 2 {
                cells.push(Cell::from(value.pad_to_width_with_alignment(
                    row_max_counts[i],
                    pad::Alignment::Right,
                )));
            } else {
                cells.push(Cell::from(value.as_str()));
            }
        }
        Row::new(cells).height(1)
    });

    let headers = ["Index", "Size", "Command"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));
    // title + header + border bottom
    let height = 3 + rows.len();
    let header = Row::new(headers).height(1);
    let mut title = " Layers ".to_string();
    if state.active == 0 {
        title = wrap_active(&title);
    }
    let t = Table::new(rows)
        .header(header)
        .block(create_block(title.as_str()))
        .widths(&[
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Min(u16::MAX),
        ]);

    let detail_widget = new_layer_detail_widget(&result.layers[3], chunks[0].width);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(height as u16),
                Constraint::Length(detail_widget.height),
                Constraint::Length(u16::MAX),
            ]
            .as_ref(),
        )
        .split(chunks[0]);
    f.render_widget(t, left_chunks[0]);
    f.render_widget(detail_widget.widget, left_chunks[1]);
}
