use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{error::Error, io};
use tui::{
    backend::Backend,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    text::{Span, Spans},
    widgets::{ListState, Paragraph},
    Frame, Terminal,
};

use crate::image::ImageAnalysisResult;

mod files;
mod image_detail;
mod layer_detail;
mod layers;
mod util;

#[derive(Default, Debug, Clone)]
struct WidgetState {
    active_list: Vec<String>,
    // 选中的区域
    active: String,
    // 被选中的层
    selected_layer: usize,
    // 层级数
    layer_count: usize,
    // 文件列表的状态
    files_state: ListState,
}

static LAYERS_WIDGET: &str = "layers";
static FILES_WIDGET: &str = "files";

impl WidgetState {
    fn next_widget(&mut self) {
        let found = self
            .active_list
            .iter()
            .position(|x| *x == self.active)
            .unwrap_or(0);
        if found >= self.active_list.len() - 1 {
            self.active = self.active_list[0].clone();
        } else {
            self.active = self.active_list[found + 1].clone();
        }
        if self.is_files_widget_active() {
            self.select_file(0);
        } else {
            self.files_state.select(None);
        }
    }
    // layers widget是否活动状态
    fn is_layers_widget_active(&self) -> bool {
        self.active == LAYERS_WIDGET
    }
    fn is_files_widget_active(&self) -> bool {
        self.active == FILES_WIDGET
    }
    fn select_file(&mut self, offset: i64) {
        let mut value = 0;
        if let Some(v) = self.files_state.selected() {
            value = v as i64;
        }
        value += offset;
        // 如果offset为0，选择第一个文件
        if value < 0 || offset == 0 {
            value = 0
        }
        self.files_state.select(Some(value as usize));
    }
    fn select_next(&mut self) {
        if self.is_files_widget_active() {
            self.select_file(1);
            return;
        }

        if self.is_layers_widget_active() && self.selected_layer < self.layer_count - 1 {
            self.selected_layer += 1;
        }
    }
    fn select_prev(&mut self) {
        if self.is_files_widget_active() {
            self.select_file(-1);
            return;
        }
        if self.is_layers_widget_active() && self.selected_layer > 0 {
            self.selected_layer -= 1;
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
        layer_count: result.layers.len(),
        // 可以选中的widget列表顺序
        active_list: vec![LAYERS_WIDGET.to_string(), FILES_WIDGET.to_string()],
        active: LAYERS_WIDGET.to_string(),
        ..Default::default()
    };
    loop {
        terminal.draw(|f| draw_widgets(f, &result, &mut state))?;

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
                // 左右均下一组件，因为只有两个组件
                KeyCode::Right => state.next_widget(),
                KeyCode::Left => state.next_widget(),
                // 组件中的上下移动
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

fn draw_widgets<B: Backend>(
    f: &mut Frame<B>,
    result: &ImageAnalysisResult,
    state: &mut WidgetState,
) {
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

    let image_detail_widget = image_detail::new_image_detail_widget(result);
    f.render_widget(image_detail_widget.widget, left_chunks[2]);

    // 文件列表
    let files_widget = files::new_files_widget(
        result,
        files::FilesWidgetOption {
            is_active: state.is_files_widget_active(),
            selected_layer: state.selected_layer,
            area: chunks[1],
        },
    );
    f.render_widget(files_widget.block, files_widget.block_area);
    f.render_widget(files_widget.content, files_widget.content_area);
    f.render_stateful_widget(
        files_widget.files,
        files_widget.files_area,
        &mut state.files_state,
    );
}
