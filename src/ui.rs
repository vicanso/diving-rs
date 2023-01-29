use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{error::Error, io, thread, time::Duration};
use tracing_subscriber::layer;
use tui::{
    backend::Backend,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Borders, Cell, Row, Table, Widget},
    Frame, Terminal,
};

use crate::image::AnalysisResult;

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

pub fn run_app(result: AnalysisResult) -> Result<(), Box<dyn Error>> {
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

fn draw_widgets<B: Backend>(f: &mut Frame<B>, result: &AnalysisResult) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        // .margin(1)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());

    let rows = result.layers.iter().map(|item| {
        let cells = vec![
            Cell::from("0"),
            Cell::from(format!("{}", item.size)),
            Cell::from(item.cmd.as_str()),
        ];
        Row::new(cells).height(1).bottom_margin(1)
    });
    let t = Table::new(rows)
        .block(Block::default().borders(Borders::ALL).title("Layers"))
        .widths(&[
            Constraint::Length(1),
            Constraint::Length(10),
            Constraint::Max(300),
        ]);
    f.render_widget(t, chunks[0]);
    //     let t = Table::new(rows)
    //     .header(header)
    //     .block(Block::default().borders(Borders::ALL).title("Table"))
    //     .highlight_style(selected_style)
    //     .highlight_symbol(">> ")
    //     .widths(&[
    //         Constraint::Percentage(50),
    //         Constraint::Length(30),
    //         Constraint::Min(10),
    //     ]);
    // f.render_stateful_widget(t, rects[0], &mut app.state);

    // for layer in result.layers  {

    // }
    // let right_chunk = chunks[1];

    // let left_chunks = Layout::default()
    //     .direction(Direction::Vertical)
    //     .constraints([Constraint::Min(3), Constraint::Min(4), Constraint::Min(5)].as_ref())
    //     .split(chunks[0]);

    // let block = Block::default().title("Layers").borders(Borders::ALL);
    // f.render_widget(block, left_chunks[0]);
    // f.render_widget(
    //     Block::default().title("Layers").borders(Borders::ALL),
    //     left_chunks[1],
    // );
    // f.render_widget(
    //     Block::default().title("Layers").borders(Borders::ALL),
    //     left_chunks[2],
    // );
    // let block = Block::default()
    //     .title(" ● Current Layer Contents")
    //     .borders(Borders::ALL);
    // f.render_widget(block, right_chunk);
}
