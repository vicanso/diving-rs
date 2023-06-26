use self::image_detail::ImageDetailWidgetOption;
use crate::image::{DockerAnalyzeResult, FileTreeItem, ImageFileSummary, ImageLayer};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::process;
use std::process::Command;
use std::sync::atomic;
use std::sync::mpsc::sync_channel;
use std::{error::Error, io};
use tui::{
    backend::Backend,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    widgets::ListState,
    Frame, Terminal,
};

mod files;
mod image_detail;
mod layer_detail;
mod layers;
mod util;

#[derive(Default, Debug, Clone)]
struct WidgetState {
    name: String,
    arch: String,
    os: String,
    active_list: Vec<String>,
    // 选中的区域
    active: String,
    selected_layer: usize,
    // 镜像大小
    size: u64,
    // 镜像解压大小
    total_size: u64,
    // 镜像层的信息
    layers: Vec<ImageLayer>,
    // 每层对应的文件树
    file_tree_list: Vec<Vec<FileTreeItem>>,
    // 镜像删除、更新等文件汇总
    file_summary_list: Vec<ImageFileSummary>,
    // 文件列表的状态
    files_state: ListState,
    // 文件列表项总数
    file_count: usize,
    // 文件树模式
    file_tree_mode: u8,
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

        if value >= self.file_count as i64 {
            return;
        }
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

        if self.is_layers_widget_active() && self.selected_layer < self.layers.len() - 1 {
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
    fn change_file_tree_mode(&mut self, mode: u8) {
        self.file_tree_mode = mode;
    }
}

pub fn run_app(result: DockerAnalyzeResult) -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let hidden = atomic::AtomicBool::default();

    // create app and run it
    let mut state = WidgetState {
        name: result.name,
        arch: result.arch,
        os: result.os,
        layers: result.layers,
        selected_layer: 0,
        file_tree_list: result.file_tree_list,
        file_summary_list: result.file_summary_list,
        size: result.size,
        total_size: result.total_size,
        // 可以选中的widget列表顺序
        active_list: vec![LAYERS_WIDGET.to_string(), FILES_WIDGET.to_string()],
        active: LAYERS_WIDGET.to_string(),
        ..Default::default()
    };
    let (tx, rx) = sync_channel(1);

    let signal = unsafe {
        signal_hook_registry::register(signal_hook::consts::SIGCONT, move || {
            // 事件触发失败则直接退出
            // 因此使用unwrap
            tx.send(true).unwrap();
        })
    }?;

    loop {
        if hidden.load(atomic::Ordering::Relaxed) {
            // 等待fg事件，出错直接退出
            // 因此使用unwrap
            rx.recv().unwrap();
            enable_raw_mode()?;
            execute!(terminal.backend_mut(), EnterAlternateScreen)?;
            terminal.hide_cursor()?;
            hidden.store(false, atomic::Ordering::Relaxed);
            terminal.clear()?;
        }
        terminal.draw(|f| draw_widgets(f, &mut state))?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                // make dev的形式下不可用
                // suspend
                KeyCode::Char('z') => {
                    // 只针对类unix系统
                    if cfg!(unix) && key.modifiers.contains(KeyModifiers::CONTROL) {
                        hidden.store(true, atomic::Ordering::Relaxed);

                        disable_raw_mode()?;
                        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                        terminal.show_cursor()?;

                        let mut kill = Command::new("kill")
                            .args(["-s", "STOP", &process::id().to_string()])
                            .spawn()?;
                        kill.wait()?;

                        continue;
                    }
                }
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
                // 文件树模式选择
                KeyCode::Char('0') => state.change_file_tree_mode(0),
                KeyCode::Char('1') => state.change_file_tree_mode(1),
                KeyCode::Char('2') => state.change_file_tree_mode(2),
                KeyCode::Esc => state.change_file_tree_mode(0),

                _ => continue,
            }
        }
    }

    // restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    signal_hook_registry::unregister(signal);
    Ok(())
}

fn draw_widgets<B: Backend>(f: &mut Frame<B>, state: &mut WidgetState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());

    let layers_widget = layers::new_layers_widget(
        &state.layers,
        layers::LayersWidgetOption {
            is_active: state.is_layers_widget_active(),
            selected_layer: state.selected_layer,
        },
    );
    let layer = state
        .layers
        .get(state.selected_layer)
        .unwrap_or_else(|| &state.layers[0]);
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
    let image_detail_widget = image_detail::new_image_detail_widget(ImageDetailWidgetOption {
        name: state.name.clone(),
        arch: state.arch.clone(),
        os: state.os.clone(),
        total_size: state.total_size,
        size: state.size,
        file_summary_list: state.file_summary_list.clone(),
    });
    f.render_widget(layers_widget.widget, left_chunks[0]);
    f.render_widget(detail_widget.widget, left_chunks[1]);
    f.render_widget(image_detail_widget.widget, left_chunks[2]);

    // 文件列表
    let files_widget = files::new_files_widget(
        &state.file_tree_list,
        files::FilesWidgetOption {
            is_active: state.is_files_widget_active(),
            selected_layer: state.selected_layer,
            area: chunks[1],
            mode: state.file_tree_mode,
        },
    );
    if state.file_count != files_widget.file_count {
        state.file_count = files_widget.file_count;
    }
    f.render_widget(files_widget.block, files_widget.block_area);
    f.render_widget(files_widget.content, files_widget.content_area);
    f.render_stateful_widget(
        files_widget.files,
        files_widget.files_area,
        &mut state.files_state,
    );
}
