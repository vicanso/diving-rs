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

mod layer_detail;
mod layers;
mod util;

#[derive(Default, Debug, Clone)]
struct WidgetState {
    // 选中的区域
    active: usize,
    // 被选中的层
    selected_layer: usize,
}

impl WidgetState {
    fn next_widget(&mut self) {
        self.active += 1;
    }
    // layers widget是否活动状态
    fn is_layers_widget_active(&self) -> bool {
        self.active == 0
    }
    fn select_next(&mut self) {
        // TODO 设置最大值
        if self.is_layers_widget_active() {
            self.selected_layer += 1;
        }
    }
    fn select_prev(&mut self) {
        if self.is_layers_widget_active() {
            if self.selected_layer > 0 {
                self.selected_layer -= 1;
            }
        }
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
                KeyCode::Down => state.select_next(),
                KeyCode::Up => state.select_prev(),
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

fn draw_widgets<B: Backend>(f: &mut Frame<B>, result: &ImageAnalysisResult, state: &WidgetState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());

    let layers_widget = layers::new_layers_widget(
        &result.layers,
        layers::LayersWidgetOption {
            is_active: state.is_layers_widget_active(),
            selected_layer: state.selected_layer,
        },
    );
    let layer = result
        .layers
        .get(state.selected_layer)
        .unwrap_or_else(|| &result.layers[0]);
    let detail_widget = layer_detail::new_layer_detail_widget(
        layer,
        layer_detail::DetailWidgetOption {
            width: chunks[0].width,
        },
    );

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(layers_widget.height),
                Constraint::Length(detail_widget.height),
                Constraint::Length(u16::MAX),
            ]
            .as_ref(),
        )
        .split(chunks[0]);
    f.render_widget(layers_widget.widget, left_chunks[0]);
    f.render_widget(detail_widget.widget, left_chunks[1]);
}
