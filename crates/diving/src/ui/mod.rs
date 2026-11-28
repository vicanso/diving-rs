use self::files::FileTreeCache;
use self::image_detail::ImageDetailWidgetOption;
use self::layers::LayerOrder;
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use diving_core::{
    fill, tr, DockerAnalyzeResult, DockerAnalyzeSummary, FileTreeItem, ImageLayer, Lang,
    Recommendation, RuntimeCompat,
};
use ratatui::{prelude::*, widgets::*};
use std::collections::HashSet;
use std::process;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::{error::Error, io};

mod files;
mod image_detail;
mod layer_detail;
mod layers;
mod util;

static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Restore terminal raw mode and alternate screen if the TUI is currently active.
pub fn restore_terminal() {
    if TUI_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

#[derive(Default, Debug)]
struct WidgetState {
    name: String,
    arch: String,
    os: String,
    active_list: Vec<&'static str>,
    active: &'static str,
    selected_layer: usize,
    layer_order: LayerOrder,
    layers_state: TableState,
    layers_page: usize,
    size: u64,
    total_size: u64,
    layers: Vec<ImageLayer>,
    file_tree_list: Vec<Vec<FileTreeItem>>,
    files_state: ListState,
    files_page: usize,
    file_count: usize,
    file_tree_mode: u8,
    search_active: bool,
    search_query: String,
    collapsed_dirs: HashSet<(usize, String)>,
    collapsed_version: u64,
    file_tree_cache: FileTreeCache,
    image_detail_scroll: u16,
    image_detail_max_scroll: u16,
    image_detail_page: u16,
    summary: DockerAnalyzeSummary,
    stale_as_of: Option<String>,
    recommendations: Vec<Recommendation>,
    base_os: String,
    runtime_compat: RuntimeCompat,
    lang: Lang,
}

const LAYERS_WIDGET: &str = "layers";
const IMAGE_DETAIL_WIDGET: &str = "image_detail";
const FILES_WIDGET: &str = "files";

impl WidgetState {
    fn cycle_widget(&mut self, delta: isize) {
        if self.active_list.is_empty() {
            return;
        }
        let len = self.active_list.len() as isize;
        let found = self
            .active_list
            .iter()
            .position(|&x| x == self.active)
            .unwrap_or(0) as isize;
        let next = (found + delta).rem_euclid(len) as usize;
        self.active = self.active_list[next];
        if self.is_files_widget_active() {
            if self.files_state.selected().is_none() && self.file_count > 0 {
                self.files_state.select(Some(0));
            }
        } else {
            self.files_state.select(None);
        }
    }

    fn next_widget(&mut self) {
        self.cycle_widget(1);
    }

    fn prev_widget(&mut self) {
        self.cycle_widget(-1);
    }

    fn is_layers_widget_active(&self) -> bool {
        self.active == LAYERS_WIDGET
    }

    fn is_image_detail_widget_active(&self) -> bool {
        self.active == IMAGE_DETAIL_WIDGET
    }

    fn is_files_widget_active(&self) -> bool {
        self.active == FILES_WIDGET
    }

    fn select_file(&mut self, offset: i64) {
        if self.file_count == 0 {
            self.files_state.select(None);
            return;
        }
        let cur = self.files_state.selected().unwrap_or(0) as i64;
        let max_idx = (self.file_count - 1) as i64;
        let next = (cur + offset).clamp(0, max_idx) as usize;
        self.files_state.select(Some(next));
    }

    fn visual_row_of_layer(&self, layer_idx: usize) -> usize {
        let len = self.layers.len();
        if len == 0 {
            return 0;
        }
        let clamped = layer_idx.min(len - 1);
        match self.layer_order {
            LayerOrder::Desc => len - 1 - clamped,
            LayerOrder::Asc => clamped,
        }
    }

    fn layer_of_visual_row(&self, row_idx: usize) -> usize {
        let len = self.layers.len();
        if len == 0 {
            return 0;
        }
        let clamped = row_idx.min(len - 1);
        match self.layer_order {
            LayerOrder::Desc => len - 1 - clamped,
            LayerOrder::Asc => clamped,
        }
    }

    fn select_layer_by(&mut self, offset: i64) {
        let max_idx = self.layers.len().saturating_sub(1) as i64;
        let cur_row = self.visual_row_of_layer(self.selected_layer) as i64;
        let next_row = (cur_row + offset).clamp(0, max_idx) as usize;
        let next = self.layer_of_visual_row(next_row);
        if next != self.selected_layer {
            self.selected_layer = next;
            self.files_state.select(None);
        }
    }

    fn move_selection(&mut self, delta: i64) {
        if self.is_files_widget_active() {
            self.select_file(delta);
        } else if self.is_layers_widget_active() {
            self.select_layer_by(delta);
        } else if self.is_image_detail_widget_active() {
            let next = (self.image_detail_scroll as i64 + delta)
                .clamp(0, self.image_detail_max_scroll as i64);
            self.image_detail_scroll = next as u16;
        }
    }

    fn page_step(&self) -> i64 {
        if self.is_files_widget_active() {
            self.files_page.max(1) as i64
        } else if self.is_layers_widget_active() {
            self.layers_page.max(1) as i64
        } else if self.is_image_detail_widget_active() {
            self.image_detail_page.max(1) as i64
        } else {
            1
        }
    }

    fn select_first(&mut self) {
        if self.is_files_widget_active() {
            if self.file_count > 0 {
                self.files_state.select(Some(0));
            }
        } else if self.is_layers_widget_active() {
            let first = self.layer_of_visual_row(0);
            if self.selected_layer != first {
                self.selected_layer = first;
                self.files_state.select(None);
            }
        } else if self.is_image_detail_widget_active() {
            self.image_detail_scroll = 0;
        }
    }

    fn select_last(&mut self) {
        if self.is_files_widget_active() {
            if self.file_count > 0 {
                self.files_state.select(Some(self.file_count - 1));
            }
        } else if self.is_layers_widget_active() {
            let last_row = self.layers.len().saturating_sub(1);
            let last = self.layer_of_visual_row(last_row);
            if self.selected_layer != last {
                self.selected_layer = last;
                self.files_state.select(None);
            }
        } else if self.is_image_detail_widget_active() {
            self.image_detail_scroll = self.image_detail_max_scroll;
        }
    }

    fn toggle_selected_dir(&mut self) {
        if !self.is_files_widget_active() {
            return;
        }
        let Some(idx) = self.files_state.selected() else {
            return;
        };
        let Some(row) = self.file_tree_cache.rows.get(idx) else {
            return;
        };
        if !row.is_dir {
            return;
        }
        let key = (self.selected_layer, row.full_path.clone());
        if !self.collapsed_dirs.remove(&key) {
            self.collapsed_dirs.insert(key);
        }
        self.collapsed_version = self.collapsed_version.wrapping_add(1);
    }

    fn change_file_tree_mode(&mut self, mode: u8) {
        self.file_tree_mode = mode;
        if self.is_files_widget_active() {
            self.files_state.select(Some(0));
        }
    }
}

pub fn run_app(result: DockerAnalyzeResult, lang: Lang) -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    TUI_ACTIVE.store(true, Ordering::SeqCst);
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let hidden = AtomicBool::default();

    let summary = result.summary();
    let layer_order = LayerOrder::default();
    let initial_layer = result
        .layers
        .iter()
        .rposition(|l| !l.empty)
        .unwrap_or_else(|| result.layers.len().saturating_sub(1));
    let initial_row = if result.layers.is_empty() {
        0
    } else {
        result.layers.len() - 1 - initial_layer
    };
    let mut layers_state = TableState::default();
    if !result.layers.is_empty() {
        layers_state.select(Some(initial_row));
    }
    let mut state = WidgetState {
        name: result.name,
        arch: result.arch,
        os: result.os,
        layers: result.layers,
        selected_layer: initial_layer,
        layer_order,
        layers_state,
        layers_page: 5,
        file_tree_list: result.file_tree_list,
        files_page: 10,
        image_detail_page: 5,
        size: result.size,
        total_size: result.total_size,
        recommendations: result.recommendations,
        base_os: result.base_os,
        runtime_compat: result.runtime_compat,
        stale_as_of: result.stale_as_of,
        lang,
        active_list: vec![LAYERS_WIDGET, IMAGE_DETAIL_WIDGET, FILES_WIDGET],
        active: LAYERS_WIDGET,
        summary,
        ..Default::default()
    };
    let (tx, rx) = sync_channel::<bool>(1);
    #[cfg(not(windows))]
    let signal = Some(unsafe {
        signal_hook_registry::register(signal_hook::consts::SIGCONT, move || {
            let _ = tx.try_send(true);
        })
    }?);
    #[cfg(windows)]
    let signal = None;

    loop {
        if hidden.load(Ordering::Relaxed) {
            if rx.recv().is_err() {
                break;
            }
            enable_raw_mode()?;
            execute!(terminal.backend_mut(), EnterAlternateScreen)?;
            TUI_ACTIVE.store(true, Ordering::SeqCst);
            terminal.hide_cursor()?;
            hidden.store(false, Ordering::Relaxed);
            terminal.clear()?;
        }
        terminal.draw(|f| draw_widgets(f, &mut state))?;

        if let Event::Key(key) = event::read()? {
            match handle_key_event(&mut state, key) {
                KeyAction::Quit => break,
                KeyAction::Suspend => {
                    hidden.store(true, Ordering::Relaxed);
                    restore_terminal();

                    let mut kill = Command::new("kill")
                        .args(["-s", "STOP", &process::id().to_string()])
                        .spawn()?;
                    kill.wait()?;
                }
                KeyAction::Continue => {}
            }
        }
    }

    restore_terminal();
    if let Some(value) = signal {
        signal_hook_registry::unregister(value);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    Continue,
    Quit,
    Suspend,
}

fn handle_key_event(state: &mut WidgetState, key: KeyEvent) -> KeyAction {
    if state.search_active {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return KeyAction::Quit;
            }
            KeyCode::Esc => {
                state.search_query.clear();
                state.search_active = false;
                if state.is_files_widget_active() {
                    state.files_state.select(Some(0));
                }
            }
            KeyCode::Enter => {
                state.search_active = false;
            }
            KeyCode::Backspace => {
                state.search_query.pop();
                if state.is_files_widget_active() {
                    state.files_state.select(Some(0));
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                state.search_query.push(c);
                if state.is_files_widget_active() {
                    state.files_state.select(Some(0));
                }
            }
            _ => {}
        }
        return KeyAction::Continue;
    }

    match key.code {
        KeyCode::Char('z') if cfg!(unix) && key.modifiers.contains(KeyModifiers::CONTROL) => {
            return KeyAction::Suspend;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return KeyAction::Quit;
        }
        KeyCode::Char('q') => return KeyAction::Quit,
        KeyCode::Tab | KeyCode::Right => state.next_widget(),
        KeyCode::Left => state.prev_widget(),
        KeyCode::Down | KeyCode::Char('j') => state.move_selection(1),
        KeyCode::Up | KeyCode::Char('k') => state.move_selection(-1),
        KeyCode::PageDown => state.move_selection(state.page_step()),
        KeyCode::PageUp => state.move_selection(-state.page_step()),
        KeyCode::Home => state.select_first(),
        KeyCode::End => state.select_last(),
        KeyCode::Enter | KeyCode::Char(' ') => state.toggle_selected_dir(),
        KeyCode::Char('/') => {
            state.active = FILES_WIDGET;
            state.search_active = true;
            if state.files_state.selected().is_none() && state.file_count > 0 {
                state.files_state.select(Some(0));
            }
        }
        KeyCode::Char('0') => state.change_file_tree_mode(0),
        KeyCode::Char('1') => state.change_file_tree_mode(1),
        KeyCode::Char('2') => state.change_file_tree_mode(2),
        KeyCode::Char('r') => state.layer_order = state.layer_order.toggle(),
        KeyCode::Esc => {
            if !state.search_query.is_empty() {
                state.search_query.clear();
            } else {
                state.change_file_tree_mode(0);
            }
        }
        _ => {}
    }
    KeyAction::Continue
}

fn draw_widgets(f: &mut Frame, state: &mut WidgetState) {
    state.selected_layer = state
        .selected_layer
        .min(state.layers.len().saturating_sub(1));
    state.layers_state.select(if state.layers.is_empty() {
        None
    } else {
        Some(state.visual_row_of_layer(state.selected_layer))
    });

    let root_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(1)])
        .split(f.area());
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(root_chunks[0]);

    let left_height = chunks[0].height;
    let min_image_detail = 8u16.min(left_height / 2);
    let fallback_layer = ImageLayer::default();
    let layer = state
        .layers
        .get(state.selected_layer)
        .unwrap_or(&fallback_layer);
    let max_detail_height = left_height.saturating_sub(min_image_detail + 4).clamp(4, 8);
    let detail_widget = layer_detail::new_layer_detail_widget(
        layer,
        layer_detail::DetailWidgetOption {
            width: chunks[0].width,
            max_height: max_detail_height,
            lang: state.lang,
        },
    );
    let max_layers_height = left_height
        .saturating_sub(detail_widget.height + min_image_detail)
        .clamp(4, 14);
    let layers_widget = layers::new_layers_widget(
        &state.layers,
        layers::LayersWidgetOption {
            is_active: state.is_layers_widget_active(),
            lang: state.lang,
            max_height: max_layers_height,
            order: state.layer_order,
        },
    );

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(layers_widget.height),
                Constraint::Length(detail_widget.height),
                Constraint::Fill(1),
            ]
            .as_ref(),
        )
        .split(chunks[0]);
    state.layers_page = left_chunks[0].height.saturating_sub(4).max(1) as usize;

    let image_detail_widget = image_detail::new_image_detail_widget(ImageDetailWidgetOption {
        is_active: state.is_image_detail_widget_active(),
        width: left_chunks[2].width,
        name: &state.name,
        arch: &state.arch,
        os: &state.os,
        total_size: state.total_size,
        size: state.size,
        summary: &state.summary,
        recommendations: &state.recommendations,
        base_os: &state.base_os,
        runtime_compat: &state.runtime_compat,
        stale_as_of: state.stale_as_of.as_deref(),
        lang: state.lang,
    });
    let visible_detail_lines = left_chunks[2].height.saturating_sub(2).max(1);
    state.image_detail_page = visible_detail_lines;
    state.image_detail_max_scroll = image_detail_widget
        .content_lines
        .saturating_sub(visible_detail_lines);
    state.image_detail_scroll = state.image_detail_scroll.min(state.image_detail_max_scroll);

    f.render_stateful_widget(
        layers_widget.widget,
        left_chunks[0],
        &mut state.layers_state,
    );
    f.render_widget(detail_widget.widget, left_chunks[1]);
    f.render_widget(
        image_detail_widget
            .widget
            .scroll((state.image_detail_scroll, 0)),
        left_chunks[2],
    );

    state
        .file_tree_cache
        .ensure_built(files::FileTreeCacheQuery {
            file_tree_list: &state.file_tree_list,
            selected_layer: state.selected_layer,
            mode: state.file_tree_mode,
            keyword: &state.search_query,
            collapsed: &state.collapsed_dirs,
            collapsed_version: state.collapsed_version,
            lang: state.lang,
        });
    let files_widget = files::new_files_widget(
        &state.file_tree_cache,
        files::FilesWidgetOption {
            is_active: state.is_files_widget_active(),
            area: chunks[1],
            mode: state.file_tree_mode,
            keyword: &state.search_query,
            lang: state.lang,
        },
    );
    state.file_count = files_widget.file_count;
    state.files_page = files_widget.files_area.height.saturating_sub(1).max(1) as usize;
    if state.file_count == 0 {
        state.files_state.select(None);
    } else if let Some(sel) = state.files_state.selected() {
        if sel >= state.file_count {
            state.files_state.select(Some(state.file_count - 1));
        }
    }
    f.render_widget(files_widget.block, files_widget.block_area);
    f.render_widget(files_widget.content, files_widget.content_area);
    f.render_stateful_widget(
        files_widget.files,
        files_widget.files_area,
        &mut state.files_state,
    );

    let footer_text = if state.search_active {
        fill(tr(state.lang, "tui.footer.search"), &[&state.search_query])
    } else {
        tr(state.lang, "tui.footer.hints").to_string()
    };
    let footer_style = if state.search_active {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };
    f.render_widget(
        Paragraph::new(footer_text).style(footer_style),
        root_chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn zero_layer_image_renders_without_panic_and_bounds_navigation() {
        let mut state = WidgetState {
            name: "empty:latest".to_string(),
            active_list: vec![LAYERS_WIDGET, IMAGE_DETAIL_WIDGET, FILES_WIDGET],
            active: LAYERS_WIDGET,
            lang: Lang::En,
            ..Default::default()
        };
        state.move_selection(1);
        state.move_selection(-1);
        state.next_widget();
        state.next_widget();
        state.move_selection(1);
        state.prev_widget();
        state.prev_widget();
        assert_eq!(state.selected_layer, 0);

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw_widgets(f, &mut state)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("(no layers)"), "{text}");
        assert!(text.contains("Image name: empty:latest"), "{text}");
    }

    #[test]
    fn many_layers_on_80x24_keep_image_details_visible() {
        let layers: Vec<ImageLayer> = (0..18)
            .map(|i| ImageLayer {
                cmd: format!("RUN step {i}"),
                size: 1024,
                ..Default::default()
            })
            .collect();
        let file_tree_list = vec![vec![]; layers.len()];
        let mut state = WidgetState {
            name: "many:latest".to_string(),
            layers,
            file_tree_list,
            active_list: vec![LAYERS_WIDGET, IMAGE_DETAIL_WIDGET, FILES_WIDGET],
            active: LAYERS_WIDGET,
            lang: Lang::En,
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw_widgets(f, &mut state)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Image Details"), "{text}");
        assert!(text.contains("Image name: many:latest"), "{text}");
        assert!(text.contains("Image efficiency score:"), "{text}");
    }

    #[test]
    fn keybindings_filter_fold_scroll_and_render_footer_hints() {
        use diving_core::Op;

        let layer0_tree = vec![FileTreeItem {
            name: "etc".into(),
            size: 300,
            children: vec![
                FileTreeItem {
                    name: "alpha.conf".into(),
                    mode: "-rw-r--r--".into(),
                    size: 100,
                    op: Op::Modified,
                    ..Default::default()
                },
                FileTreeItem {
                    name: "beta.txt".into(),
                    mode: "-rw-r--r--".into(),
                    size: 200,
                    op: Op::None,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];
        let mut recs = Vec::new();
        for i in 0..8 {
            recs.push(Recommendation {
                id: format!("r{i}"),
                category: "size".into(),
                severity: "medium".into(),
                title: format!("Recommendation item number {i} for scroll testing"),
                ..Default::default()
            });
        }
        let mut state = WidgetState {
            name: "ux:latest".into(),
            layers: vec![
                ImageLayer {
                    cmd: "RUN layer 0".into(),
                    size: 100,
                    ..Default::default()
                },
                ImageLayer {
                    cmd: "RUN layer 1".into(),
                    size: 200,
                    ..Default::default()
                },
            ],
            file_tree_list: vec![layer0_tree, vec![]],
            recommendations: recs,
            active_list: vec![LAYERS_WIDGET, IMAGE_DETAIL_WIDGET, FILES_WIDGET],
            active: LAYERS_WIDGET,
            selected_layer: 1,
            lang: Lang::En,
            ..Default::default()
        };
        let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let draw_text = |term: &mut Terminal<TestBackend>, state: &mut WidgetState| -> String {
            term.draw(|f| draw_widgets(f, state)).unwrap();
            let buf = term.backend().buffer().clone();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        // Hint bar shown on initial draw; `j`/`End`/`Home` navigate layers in Desc order (`1` on top).
        let initial = draw_text(&mut term, &mut state);
        assert!(initial.contains("q Quit"), "{initial}");
        assert!(initial.contains("r Order"), "{initial}");
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(state.selected_layer, 0);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Home));
        assert_eq!(state.selected_layer, 1);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::End));
        assert_eq!(state.selected_layer, 0);

        // `r` toggles to Asc order (`0` on top), reversing visual stepping cleanly.
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Char('r')));
        assert_eq!(state.layer_order, LayerOrder::Asc);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(state.selected_layer, 1);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Home));
        assert_eq!(state.selected_layer, 0);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::End));
        assert_eq!(state.selected_layer, 1);

        // Toggle back to Desc and jump to bottom (`0`) before searching its file tree.
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Char('r')));
        assert_eq!(state.layer_order, LayerOrder::Desc);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::End));
        assert_eq!(state.selected_layer, 0);

        // Tab to Image Details and verify scroll advances/returns cleanly.
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Tab));
        assert!(state.is_image_detail_widget_active());
        let _ = draw_text(&mut term, &mut state);
        assert!(state.image_detail_max_scroll > 0);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::PageDown));
        assert!(state.image_detail_scroll > 0);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Home));
        assert_eq!(state.image_detail_scroll, 0);

        // `/` activates Files search mode, filters rows, `Enter` keeps filter, `Esc` clears.
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Char('/')));
        assert!(state.is_files_widget_active());
        assert!(state.search_active);
        for ch in "alpha".chars() {
            handle_key_event(&mut state, KeyEvent::from(KeyCode::Char(ch)));
        }
        let filtered = draw_text(&mut term, &mut state);
        assert!(filtered.contains("Filter: /alpha_"), "{filtered}");
        assert_eq!(state.file_count, 2); // `etc` + `alpha.conf`
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Enter));
        assert!(!state.search_active);
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Esc));
        let _ = draw_text(&mut term, &mut state);
        assert_eq!(state.file_count, 3); // `etc`, `alpha.conf`, `beta.txt`

        // `Enter` collapses directory `etc` down to 1 row, `Space` re-expands.
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Home));
        assert_eq!(state.files_state.selected(), Some(0));
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Enter));
        let collapsed = draw_text(&mut term, &mut state);
        assert_eq!(state.file_count, 1);
        assert!(collapsed.contains("▸ etc"), "{collapsed}");
        handle_key_event(&mut state, KeyEvent::from(KeyCode::Char(' ')));
        let _ = draw_text(&mut term, &mut state);
        assert_eq!(state.file_count, 3);
    }
}
