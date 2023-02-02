use bytesize::ByteSize;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pad::{Alignment, PadStr};
use std::{cell, error::Error, io, thread, time::Duration, vec};
use tracing_subscriber::layer;
use tui::{
    backend::Backend,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Widget},
    Frame, Terminal,
};

use crate::image::ImageAnalysisResult;

// 分割显示区域
fn split_layer<B: Backend>(f: &mut Frame<B>) -> Vec<Rect> {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        // .margin(1)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Min(4), Constraint::Min(5)].as_ref())
        .split(chunks[0]);
    vec![left_chunks[0], chunks[1], left_chunks[1], left_chunks[2]]
}

pub fn run_app(result: ImageAnalysisResult) -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    loop {
        terminal.draw(|f| draw_widgets(f, &result))?;

        if let Event::Key(key) = event::read()? {
            // ctrl + c
            if key.eq(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)) {
                break;
            }
            // q 退出
            if let KeyCode::Char('q') = key.code {
                break;
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

fn draw_widgets<B: Backend>(f: &mut Frame<B>, result: &ImageAnalysisResult) {
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
                cells.push(Cell::from(
                    value.pad_to_width_with_alignment(row_max_counts[i], Alignment::Right),
                ));
            } else {
                cells.push(Cell::from(value.as_str()));
            }
        }
        Row::new(cells).height(1)
    });
    let headers = ["Index", "Size", "Command"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));
    let header = Row::new(headers).height(1);
    let t = Table::new(rows)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" ● Layers"))
        .widths(&[
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Min(500),
        ]);
    f.render_widget(t, chunks[0]);
}
