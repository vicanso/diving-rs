use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{error::Error, io, thread, time::Duration};
use tui::{
    backend::Backend,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Borders, Widget},
    Frame, Terminal,
};

use crate::image::DockerAnalysisResult;

struct AppState {
    focus: u8,
    chunks: Vec<Rect>,
}

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

pub fn run_app(result: DockerAnalysisResult) -> Result<(), Box<dyn Error>> {
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

fn draw_widgets<B: Backend>(f: &mut Frame<B>, result: &DockerAnalysisResult) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        // .margin(1)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());
    let right_chunk = chunks[1];

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Min(4), Constraint::Min(5)].as_ref())
        .split(chunks[0]);

    let block = Block::default().title("Layers").borders(Borders::ALL);
    f.render_widget(block, left_chunks[0]);
    f.render_widget(
        Block::default().title("Layers").borders(Borders::ALL),
        left_chunks[1],
    );
    f.render_widget(
        Block::default().title("Layers").borders(Borders::ALL),
        left_chunks[2],
    );
    let block = Block::default()
        .title(" ● Current Layer Contents")
        .borders(Borders::ALL);
    f.render_widget(block, right_chunk);
}
