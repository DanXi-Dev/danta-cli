mod image_parse;
mod image_render;

use std::collections::{HashMap, HashSet};
use std::io::stdout;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{prelude::*, widgets::*};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use tokio::sync::mpsc;

use crate::api::DantaClient;
use crate::config::{BorderStyle, DantaConfig, ThumbnailRenderMode};
use crate::models::*;

// ── ASCII Art Banner (rowancap font, same as pikpaktui) ──

const LOGO: [&str; 5] = [
    r#"dMMMMb  .aMMMb  dMMMMb dMMMMMMP .aMMMb  "#,
    r#"dMP VMP dMP"dMP dMP dMP   dMP   dMP"dMP "#,
    r#"dMP dMP dMMMMMP dMP dMP   dMP   dMMMMMP  "#,
    r#"dMP.aMP dMP dMP dMP dMP   dMP   dMP dMP  "#,
    r#"dMMMMP" dMP dMP dMP dMP   dMP   dMP dMP  "#,
];

const LOGO_COLORS: [Color; 5] = [
    Color::LightCyan,
    Color::Cyan,
    Color::LightBlue,
    Color::Blue,
    Color::LightMagenta,
];

// ── Helpers ──

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}

fn styled_help_spans(pairs: &[(&str, &str)]) -> Vec<Span<'static>> {
    let key_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::DarkGray);
    let sep_style = Style::default().fg(Color::DarkGray);

    let mut spans = Vec::new();
    for (i, (key, desc)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" \u{2022} ", sep_style));
        }
        spans.push(Span::styled(key.to_string(), key_style));
        spans.push(Span::styled(format!(" {}", desc), desc_style));
    }
    spans
}

fn hint_line(hints: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw("  ")];
    spans.extend(styled_help_spans(hints));
    Line::from(spans)
}

// ── App State ──

#[derive(PartialEq)]
enum View {
    HoleList,
    HoleDetail,
    Input,
    MessageList,
}

#[derive(PartialEq, Clone)]
enum InputMode {
    NewPost,
    Reply,
}

struct SettingsState {
    selected: usize,
    editing: bool,
    draft: DantaConfig,
    modified: bool,
}

struct ImageResult {
    url: String,
    image: anyhow::Result<image::DynamicImage>,
}

struct ImageViewerState {
    urls: Vec<(String, String)>, // (alt_text, url)
    current_index: usize,
}

impl PartialEq for ImageViewerState {
    fn eq(&self, _other: &Self) -> bool { false }
}

#[derive(PartialEq)]
enum Overlay {
    Search(SearchOverlay),
    Help,
    DivisionPicker,
    Settings,
    ImageViewer(ImageViewerState),
}

struct SearchOverlay {
    query: String,
    cursor: usize,
    results: Vec<Floor>,
    list_state: ListState,
    active: bool,
    has_results: bool,
}

impl SearchOverlay {
    fn new() -> Self {
        Self {
            query: String::new(),
            cursor: 0,
            results: vec![],
            list_state: ListState::default(),
            active: true,
            has_results: false,
        }
    }
}

// For PartialEq on Overlay
impl PartialEq for SearchOverlay {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

struct App {
    client: DantaClient,
    config: DantaConfig,
    view: View,
    prev_view: Option<View>,
    // Hole list
    divisions: Vec<Division>,
    current_division: usize,
    holes: Vec<Hole>,
    hole_list_state: ListState,
    loading_more_holes: bool,
    // Hole detail
    current_hole: Option<Hole>,
    floors: Vec<Floor>,
    floor_scroll: u16,
    floor_selected: usize,
    floors_all_loaded: bool,
    // Input (post/reply)
    input_mode: InputMode,
    input_buf: String,
    input_cursor: usize,
    input_tag_buf: String,
    // Overlay
    overlay: Option<Overlay>,
    division_picker_state: ListState,
    settings: SettingsState,
    // Messages
    messages: Vec<Message>,
    message_list_state: ListState,
    // Images
    image_cache: HashMap<String, image::DynamicImage>,
    image_loading: HashSet<String>,
    image_tx: mpsc::UnboundedSender<ImageResult>,
    image_rx: mpsc::UnboundedReceiver<ImageResult>,
    picker: Option<Picker>,
    protocol_cache: HashMap<String, StatefulProtocol>,
    // Status
    status: String,
    should_quit: bool,
}

impl App {
    fn new(client: DantaClient, config: DantaConfig) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        let (image_tx, image_rx) = mpsc::unbounded_channel();
        Self {
            client,
            config: config.clone(),
            view: View::HoleList,
            prev_view: None,
            divisions: vec![],
            current_division: 0,
            holes: vec![],
            hole_list_state: state,
            loading_more_holes: false,
            current_hole: None,
            floors: vec![],
            floor_scroll: 0,
            floor_selected: 0,
            floors_all_loaded: false,
            input_mode: InputMode::NewPost,
            input_buf: String::new(),
            input_cursor: 0,
            input_tag_buf: String::new(),
            overlay: None,
            division_picker_state: ListState::default(),
            settings: SettingsState {
                selected: 0,
                editing: false,
                draft: config,
                modified: false,
            },
            messages: vec![],
            message_list_state: ListState::default(),
            image_cache: HashMap::new(),
            image_loading: HashSet::new(),
            image_tx,
            image_rx,
            picker: None,
            protocol_cache: HashMap::new(),
            status: "Loading...".into(),
            should_quit: false,
        }
    }

    fn selected_hole(&self) -> Option<&Hole> {
        self.hole_list_state
            .selected()
            .and_then(|i| self.holes.get(i))
    }

    fn selected_floor(&self) -> Option<&Floor> {
        self.floors.get(self.floor_selected)
    }

    fn division_name(&self) -> &str {
        self.divisions
            .get(self.current_division)
            .map(|d| d.name.as_str())
            .unwrap_or("--")
    }

    fn division_id(&self) -> i64 {
        self.divisions
            .get(self.current_division)
            .map(|d| d.id)
            .unwrap_or(1)
    }

    fn enter_input(&mut self, mode: InputMode) {
        self.input_buf.clear();
        self.input_cursor = 0;
        self.input_tag_buf.clear();
        self.input_mode = mode;
        self.prev_view = Some(std::mem::replace(&mut self.view, View::Input));
    }

    fn help_pairs(&self) -> Vec<(&'static str, &'static str)> {
        match self.view {
            View::HoleList => vec![
                ("j/k", "nav"),
                ("Enter", "open"),
                ("n", "new post"),
                ("/", "search"),
                ("Tab", "division"),
                ("m", "msg"),
                (",", "settings"),
                ("h", "help"),
                ("q", "quit"),
            ],
            View::HoleDetail => vec![
                ("j/k", "nav"),
                ("r", "reply"),
                ("i", "image"),
                ("l/d", "like/dislike"),
                ("f/F", "fav/unfav"),
                ("s/S", "sub/unsub"),
                ("/", "search"),
                ("h", "help"),
                ("q", "back"),
            ],
            View::Input => vec![
                ("Enter", "submit"),
                ("Alt+Enter", "newline"),
                ("Tab", "tags"),
                ("Esc", "cancel"),
            ],
            View::MessageList => vec![
                ("j/k", "nav"),
                ("Enter", "mark read"),
                ("c", "clear all"),
                ("q", "back"),
            ],
        }
    }
}

fn styled_block(config: &DantaConfig) -> Block<'static> {
    let block = Block::default().borders(Borders::ALL);
    match config.border_style {
        BorderStyle::Rounded => block.border_type(BorderType::Rounded),
        BorderStyle::Double => block.border_type(BorderType::Double),
        BorderStyle::Thick => block.border_type(BorderType::Thick),
    }
}

fn clear_overlay_area(f: &mut Frame, area: Rect) {
    let full = f.area();
    let extended = Rect {
        x: area.x.saturating_sub(1),
        y: area.y,
        width: area.width + if area.x > 0 { 2 } else { 1 },
        height: area.height,
    };
    f.render_widget(Clear, extended.intersection(full));
}

fn open_settings(app: &mut App) {
    app.settings.draft = app.config.clone();
    app.settings.selected = 0;
    app.settings.editing = false;
    app.settings.modified = false;
    app.overlay = Some(Overlay::Settings);
}

fn spawn_image_fetch(
    tx: mpsc::UnboundedSender<ImageResult>,
    http: reqwest::Client,
    auth: Option<String>,
    url: String,
) {
    tokio::spawn(async move {
        let result = fetch_image(&http, auth.as_deref(), &url).await;
        let _ = tx.send(ImageResult { url, image: result });
    });
}

async fn fetch_image(
    http: &reqwest::Client,
    auth: Option<&str>,
    url: &str,
) -> anyhow::Result<image::DynamicImage> {
    use image::ImageReader;
    use std::io::Cursor;

    let mut req = http.get(url);
    if let Some(token) = auth {
        req = req.header("Authorization", token);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    let bytes = resp.bytes().await?;
    let img = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()?
        .decode()?;
    Ok(img)
}

pub async fn run(client: DantaClient) -> Result<()> {
    let config = DantaConfig::load();
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut app = App::new(client, config);

    // Initialize image protocol picker (Kitty/iTerm2/Sixel detection)
    if let Ok(mut picker) = Picker::from_query_stdio() {
        use ratatui_image::picker::ProtocolType;
        match app.config.image_protocol {
            crate::config::ImageProtocol::Auto => {
                // Fix iTerm2 misdetection as Kitty
                if picker.protocol_type() == ProtocolType::Kitty {
                    if let Ok(tp) = std::env::var("TERM_PROGRAM") {
                        if tp.contains("iTerm") {
                            picker.set_protocol_type(ProtocolType::Iterm2);
                        }
                    }
                }
            }
            crate::config::ImageProtocol::Kitty => picker.set_protocol_type(ProtocolType::Kitty),
            crate::config::ImageProtocol::Iterm2 => picker.set_protocol_type(ProtocolType::Iterm2),
            crate::config::ImageProtocol::Sixel => picker.set_protocol_type(ProtocolType::Sixel),
        }
        app.picker = Some(picker);
    }

    match app.client.get_divisions().await {
        Ok(divs) => {
            // Set current_division to match config.default_division
            let default_div = app.config.default_division;
            let idx = divs.iter().position(|d| d.id == default_div).unwrap_or(0);
            app.divisions = divs;
            app.current_division = idx;
        }
        Err(e) => app.status = format!("Error: {}", e),
    }
    load_holes(&mut app).await;

    let mut needs_redraw = true;

    loop {
        // Poll completed image fetches
        while let Ok(result) = app.image_rx.try_recv() {
            app.image_loading.remove(&result.url);
            match result.image {
                Ok(img) => { app.image_cache.insert(result.url, img); }
                Err(e) => { app.status = format!("Image error: {}", e); }
            }
            needs_redraw = true;
        }

        if needs_redraw {
            terminal.draw(|f| ui(f, &mut app))?;
            needs_redraw = false;
        }

        // Poll less frequently when idle (no pending images)
        let poll_timeout = if app.image_loading.is_empty() {
            std::time::Duration::from_secs(1)
        } else {
            std::time::Duration::from_millis(50)
        };

        if event::poll(poll_timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    needs_redraw = true;

                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.should_quit = true;
                    }

                    // Overlay takes priority
                    if app.overlay.is_some() {
                        handle_overlay_keys(&mut app, key.code, key.modifiers).await;
                    } else {
                        match app.view {
                            View::HoleList => handle_hole_list_keys(&mut app, key.code).await,
                            View::HoleDetail => {
                                handle_hole_detail_keys(&mut app, key.code).await
                            }
                            View::Input => {
                                handle_input_keys(&mut app, key.code, key.modifiers).await
                            }
                            View::MessageList => {
                                handle_message_keys(&mut app, key.code).await
                            }
                        }
                    }
                }
                Event::Resize(..) => {
                    app.protocol_cache.clear();
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

// ── Data loading ──

async fn load_holes(app: &mut App) {
    app.status = format!("Loading {}...", app.division_name());
    let div_id = app.division_id();
    match app
        .client
        .get_holes(div_id, None, 10, app.config.sort_order.as_str())
        .await
    {
        Ok(holes) => {
            app.holes = holes;
            app.hole_list_state.select(Some(0));
            update_hole_list_status(app);
        }
        Err(e) => app.status = format!("Error: {}", e),
    }
}

async fn load_more_holes(app: &mut App) {
    if app.loading_more_holes {
        return;
    }
    let cursor = match app.holes.last() {
        Some(h) => h.time_updated.clone(),
        None => return,
    };
    app.loading_more_holes = true;
    app.status = "Loading more...".into();
    let div_id = app.division_id();
    match app
        .client
        .get_holes(div_id, Some(&cursor), 10, app.config.sort_order.as_str())
        .await
    {
        Ok(new_holes) => {
            if new_holes.is_empty() {
                app.status = "No more holes.".into();
            } else {
                app.holes.extend(new_holes);
                update_hole_list_status(app);
            }
        }
        Err(e) => app.status = format!("Error: {}", e),
    }
    app.loading_more_holes = false;
}

fn update_hole_list_status(app: &mut App) {
    let pos = app.hole_list_state.selected().unwrap_or(0) + 1;
    let total = app.holes.len();
    app.status = format!("{} [{}/{}]", app.division_name(), pos, total);
}

async fn load_hole_detail(app: &mut App, hole_id: i64) {
    app.status = format!("Loading hole #{}...", hole_id);
    match app.client.get_hole(hole_id).await {
        Ok(hole) => app.current_hole = Some(hole),
        Err(e) => {
            app.status = format!("Error: {}", e);
            return;
        }
    }
    match app.client.get_floors(hole_id, 0, app.config.floors_per_page, None).await {
        Ok(floors) => {
            app.floors_all_loaded = (floors.len() as u32) < app.config.floors_per_page;
            prefetch_floor_images(app, &floors);
            app.floors = floors;
            app.floor_scroll = 0;
            app.floor_selected = 0;
            app.view = View::HoleDetail;
            update_detail_status(app);
        }
        Err(e) => app.status = format!("Error: {}", e),
    }
}

async fn load_more_floors(app: &mut App) {
    if app.floors_all_loaded {
        return;
    }
    let hole_id = match &app.current_hole {
        Some(h) => h.id,
        None => return,
    };
    let offset = app.floors.len() as u32;
    match app.client.get_floors(hole_id, offset, app.config.floors_per_page, None).await {
        Ok(new_floors) => {
            if (new_floors.len() as u32) < app.config.floors_per_page {
                app.floors_all_loaded = true;
            }
            prefetch_floor_images(app, &new_floors);
            app.floors.extend(new_floors);
            update_detail_status(app);
        }
        Err(e) => app.status = format!("Error: {}", e),
    }
}

fn update_detail_status(app: &mut App) {
    let hole_id = app.current_hole.as_ref().map(|h| h.id).unwrap_or(0);
    let total_reply = app.current_hole.as_ref().map(|h| h.reply).unwrap_or(0);
    let loaded_marker = if app.floors_all_loaded { "" } else { "+" };
    app.status = format!(
        "Hole #{} [{}/{}{}] ({}\u{56de}\u{590d})",
        hole_id,
        app.floor_selected + 1,
        app.floors.len(),
        loaded_marker,
        total_reply,
    );
}

fn prefetch_floor_images(app: &mut App, floors: &[Floor]) {
    for floor in floors {
        for (_, url) in image_parse::extract_image_urls(&floor.content) {
            if !app.image_cache.contains_key(&url) && !app.image_loading.contains(&url) {
                app.image_loading.insert(url.clone());
                spawn_image_fetch(
                    app.image_tx.clone(),
                    app.client.http().clone(),
                    app.client.auth_value(),
                    url,
                );
            }
        }
    }
}

// ── Key handlers ──

async fn handle_hole_list_keys(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Down | KeyCode::Char('j') => {
            let i = app.hole_list_state.selected().unwrap_or(0);
            if i < app.holes.len().saturating_sub(1) {
                app.hole_list_state.select(Some(i + 1));
                update_hole_list_status(app);
            } else {
                load_more_holes(app).await;
                if i < app.holes.len().saturating_sub(1) {
                    app.hole_list_state.select(Some(i + 1));
                    update_hole_list_status(app);
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.hole_list_state.selected().unwrap_or(0);
            if i > 0 {
                app.hole_list_state.select(Some(i - 1));
                update_hole_list_status(app);
            }
        }
        KeyCode::Char('G') => {
            if !app.holes.is_empty() {
                app.hole_list_state
                    .select(Some(app.holes.len().saturating_sub(1)));
                update_hole_list_status(app);
            }
        }
        KeyCode::Char('g') => {
            if !app.holes.is_empty() {
                app.hole_list_state.select(Some(0));
                update_hole_list_status(app);
            }
        }
        KeyCode::Enter => {
            if let Some(hole) = app.selected_hole() {
                let id = hole.id;
                load_hole_detail(app, id).await;
            }
        }
        KeyCode::Tab => {
            app.division_picker_state
                .select(Some(app.current_division));
            app.overlay = Some(Overlay::DivisionPicker);
        }
        KeyCode::Char('r') => load_holes(app).await,
        KeyCode::Char('n') => app.enter_input(InputMode::NewPost),
        KeyCode::Char('/') => {
            app.overlay = Some(Overlay::Search(SearchOverlay::new()));
        }
        KeyCode::Char('h') | KeyCode::Char('?') => {
            app.overlay = Some(Overlay::Help);
        }
        KeyCode::Char('m') => {
            match app.client.get_messages(false).await {
                Ok(msgs) => {
                    app.messages = msgs;
                    app.message_list_state.select(if app.messages.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    app.view = View::MessageList;
                    app.status = format!("{} messages", app.messages.len());
                }
                Err(e) => app.status = format!("Error: {}", e),
            }
        }
        KeyCode::Char(',') => open_settings(app),
        _ => {}
    }
}

async fn handle_hole_detail_keys(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => {
            app.view = View::HoleList;
            app.current_hole = None;
            app.floors.clear();
            app.protocol_cache.clear();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.floor_selected < app.floors.len().saturating_sub(1) {
                app.floor_selected += 1;
                update_detail_status(app);
            } else if !app.floors_all_loaded {
                load_more_floors(app).await;
                if app.floor_selected < app.floors.len().saturating_sub(1) {
                    app.floor_selected += 1;
                    update_detail_status(app);
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.floor_selected > 0 {
                app.floor_selected -= 1;
                update_detail_status(app);
            }
        }
        KeyCode::PageDown => {
            app.floor_scroll = app.floor_scroll.saturating_add(15);
        }
        KeyCode::PageUp => {
            app.floor_scroll = app.floor_scroll.saturating_sub(15);
        }
        KeyCode::Char('G') => {
            if !app.floors.is_empty() {
                app.floor_selected = app.floors.len() - 1;
                update_detail_status(app);
            }
        }
        KeyCode::Char('g') => {
            app.floor_selected = 0;
            app.floor_scroll = 0;
            update_detail_status(app);
        }
        KeyCode::Char('r') => app.enter_input(InputMode::Reply),
        KeyCode::Char('/') => {
            app.overlay = Some(Overlay::Search(SearchOverlay::new()));
        }
        KeyCode::Char('h') | KeyCode::Char('?') => {
            app.overlay = Some(Overlay::Help);
        }
        KeyCode::Char('l') => {
            if let Some(floor) = app.selected_floor() {
                let fid = floor.id;
                let val = if floor.liked { 0 } else { 1 };
                match app.client.like_floor(fid, val).await {
                    Ok(f) => {
                        let action = if val == 1 { "Liked" } else { "Unliked" };
                        app.floors[app.floor_selected] = f;
                        app.status = format!("{} floor #{}", action, fid);
                    }
                    Err(e) => app.status = format!("Error: {}", e),
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(floor) = app.selected_floor() {
                let fid = floor.id;
                let val = if floor.disliked { 0 } else { -1 };
                match app.client.like_floor(fid, val).await {
                    Ok(f) => {
                        let action = if val == -1 { "Disliked" } else { "Undisliked" };
                        app.floors[app.floor_selected] = f;
                        app.status = format!("{} floor #{}", action, fid);
                    }
                    Err(e) => app.status = format!("Error: {}", e),
                }
            }
        }
        KeyCode::Char('f') => {
            if let Some(hole) = &app.current_hole {
                let hid = hole.id;
                match app.client.add_favorite(hid).await {
                    Ok(()) => app.status = format!("Favorited hole #{}", hid),
                    Err(e) => app.status = format!("Error: {}", e),
                }
            }
        }
        KeyCode::Char('F') => {
            if let Some(hole) = &app.current_hole {
                let hid = hole.id;
                match app.client.remove_favorite(hid).await {
                    Ok(()) => app.status = format!("Unfavorited hole #{}", hid),
                    Err(e) => app.status = format!("Error: {}", e),
                }
            }
        }
        KeyCode::Char('s') => {
            if let Some(hole) = &app.current_hole {
                let hid = hole.id;
                match app.client.add_subscription(hid).await {
                    Ok(()) => app.status = format!("Subscribed #{}", hid),
                    Err(e) => app.status = format!("Error: {}", e),
                }
            }
        }
        KeyCode::Char('S') => {
            if let Some(hole) = &app.current_hole {
                let hid = hole.id;
                match app.client.remove_subscription(hid).await {
                    Ok(()) => app.status = format!("Unsubscribed #{}", hid),
                    Err(e) => app.status = format!("Error: {}", e),
                }
            }
        }
        KeyCode::Char('i') => {
            if let Some(floor) = app.selected_floor() {
                let urls = image_parse::extract_image_urls(&floor.content);
                if !urls.is_empty() {
                    for (_, url) in &urls {
                        if !app.image_cache.contains_key(url) && !app.image_loading.contains(url) {
                            app.image_loading.insert(url.clone());
                            spawn_image_fetch(app.image_tx.clone(), app.client.http().clone(), app.client.auth_value(), url.clone());
                        }
                    }
                    app.overlay = Some(Overlay::ImageViewer(ImageViewerState {
                        urls,
                        current_index: 0,
                    }));
                } else {
                    app.status = "No images in this floor.".into();
                }
            }
        }
        KeyCode::Char(',') => open_settings(app),
        _ => {}
    }
}

async fn handle_overlay_keys(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    let overlay = app.overlay.take();
    match overlay {
        Some(Overlay::Help) => {
            // Any key closes help
        }
        Some(Overlay::DivisionPicker) => {
            handle_division_picker_keys(app, key).await;
        }
        Some(Overlay::Search(search)) => {
            app.overlay = Some(Overlay::Search(search));
            handle_search_keys(app, key, modifiers).await;
        }
        Some(Overlay::Settings) => {
            app.overlay = Some(Overlay::Settings);
            handle_settings_keys(app, key);
        }
        Some(Overlay::ImageViewer(mut state)) => {
            match key {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('i') => {
                    app.protocol_cache.retain(|k, _| !k.starts_with("__viewer__"));
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('j') => {
                    if state.current_index < state.urls.len().saturating_sub(1) {
                        state.current_index += 1;
                        // Trigger fetch for new image if needed
                        let (_, url) = &state.urls[state.current_index];
                        if !app.image_cache.contains_key(url) && !app.image_loading.contains(url) {
                            app.image_loading.insert(url.clone());
                            spawn_image_fetch(app.image_tx.clone(), app.client.http().clone(), app.client.auth_value(), url.clone());
                        }
                    }
                    app.overlay = Some(Overlay::ImageViewer(state));
                }
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('k') => {
                    if state.current_index > 0 {
                        state.current_index -= 1;
                    }
                    app.overlay = Some(Overlay::ImageViewer(state));
                }
                _ => {
                    app.overlay = Some(Overlay::ImageViewer(state));
                }
            }
        }
        None => {}
    }
}

// Settings items: 0=border, 1=help_bar, 2=ascii_art, 3=sort, 4=floors/page, 5=search_size, 6=default_div, 7=thumbnail, 8=image_protocol
const SETTINGS_COUNT: usize = 9;

fn handle_settings_keys(app: &mut App, key: KeyCode) {
    let s = &mut app.settings;
    if s.editing {
        match s.selected {
            0 => {
                // Border style: cycle
                match key {
                    KeyCode::Left => { s.draft.border_style = s.draft.border_style.prev(); s.modified = true; }
                    KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter => { s.draft.border_style = s.draft.border_style.next(); s.modified = true; }
                    KeyCode::Esc => { s.editing = false; }
                    _ => {}
                }
            }
            1 => {
                // Show help bar: toggle
                match key {
                    KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Left | KeyCode::Right => {
                        s.draft.show_help_bar = !s.draft.show_help_bar;
                        s.modified = true;
                        s.editing = false;
                    }
                    KeyCode::Esc => { s.editing = false; }
                    _ => {}
                }
            }
            2 => {
                // Show ASCII art: toggle
                match key {
                    KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Left | KeyCode::Right => {
                        s.draft.show_ascii_art = !s.draft.show_ascii_art;
                        s.modified = true;
                        s.editing = false;
                    }
                    KeyCode::Esc => { s.editing = false; }
                    _ => {}
                }
            }
            3 => {
                // Sort order: cycle
                match key {
                    KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter => {
                        s.draft.sort_order = s.draft.sort_order.next();
                        s.modified = true;
                    }
                    KeyCode::Esc => { s.editing = false; }
                    _ => {}
                }
            }
            4 => {
                // Floors per page: +/-
                match key {
                    KeyCode::Right | KeyCode::Up => {
                        s.draft.floors_per_page = (s.draft.floors_per_page + 10).min(200);
                        s.modified = true;
                    }
                    KeyCode::Left | KeyCode::Down => {
                        s.draft.floors_per_page = s.draft.floors_per_page.saturating_sub(10).max(10);
                        s.modified = true;
                    }
                    KeyCode::Esc | KeyCode::Enter => { s.editing = false; }
                    _ => {}
                }
            }
            5 => {
                // Search page size: +/-
                match key {
                    KeyCode::Right | KeyCode::Up => {
                        s.draft.search_page_size = (s.draft.search_page_size + 5).min(50);
                        s.modified = true;
                    }
                    KeyCode::Left | KeyCode::Down => {
                        s.draft.search_page_size = s.draft.search_page_size.saturating_sub(5).max(5);
                        s.modified = true;
                    }
                    KeyCode::Esc | KeyCode::Enter => { s.editing = false; }
                    _ => {}
                }
            }
            6 => {
                // Default division: +/-
                match key {
                    KeyCode::Right | KeyCode::Up => {
                        s.draft.default_division = (s.draft.default_division + 1).min(5);
                        s.modified = true;
                    }
                    KeyCode::Left | KeyCode::Down => {
                        s.draft.default_division = (s.draft.default_division - 1).max(1);
                        s.modified = true;
                    }
                    KeyCode::Esc | KeyCode::Enter => { s.editing = false; }
                    _ => {}
                }
            }
            7 => {
                // Thumbnail mode: cycle
                match key {
                    KeyCode::Left => { s.draft.thumbnail_mode = s.draft.thumbnail_mode.prev(); s.modified = true; }
                    KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter => { s.draft.thumbnail_mode = s.draft.thumbnail_mode.next(); s.modified = true; }
                    KeyCode::Esc => { s.editing = false; }
                    _ => {}
                }
            }
            8 => {
                // Image protocol: cycle
                match key {
                    KeyCode::Left => { s.draft.image_protocol = s.draft.image_protocol.prev(); s.modified = true; }
                    KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter => { s.draft.image_protocol = s.draft.image_protocol.next(); s.modified = true; }
                    KeyCode::Esc => { s.editing = false; }
                    _ => {}
                }
            }
            _ => { s.editing = false; }
        }
    } else {
        match key {
            KeyCode::Down | KeyCode::Char('j') => {
                s.selected = (s.selected + 1).min(SETTINGS_COUNT - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                s.selected = s.selected.saturating_sub(1);
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                s.editing = true;
            }
            KeyCode::Char('s') => {
                if s.modified {
                    match s.draft.save() {
                        Ok(()) => {
                            app.config = s.draft.clone();
                            app.status = "Settings saved.".into();
                            app.overlay = None;
                        }
                        Err(e) => app.status = format!("Save failed: {}", e),
                    }
                    return;
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                app.overlay = None;
                return;
            }
            _ => {}
        }
    }
}

async fn handle_division_picker_keys(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Tab => {
            // Close without changing
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let i = app.division_picker_state.selected().unwrap_or(0);
            if i < app.divisions.len().saturating_sub(1) {
                app.division_picker_state.select(Some(i + 1));
            }
            app.overlay = Some(Overlay::DivisionPicker);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.division_picker_state.selected().unwrap_or(0);
            if i > 0 {
                app.division_picker_state.select(Some(i - 1));
            }
            app.overlay = Some(Overlay::DivisionPicker);
        }
        KeyCode::Enter => {
            if let Some(i) = app.division_picker_state.selected() {
                app.current_division = i;
                load_holes(app).await;
            }
        }
        _ => {
            app.overlay = Some(Overlay::DivisionPicker);
        }
    }
}

async fn handle_search_keys(app: &mut App, key: KeyCode, _modifiers: KeyModifiers) {
    let search = match &mut app.overlay {
        Some(Overlay::Search(s)) => s,
        _ => return,
    };

    if search.active {
        match key {
            KeyCode::Esc => {
                app.overlay = None;
            }
            KeyCode::Enter => {
                if search.query.trim().is_empty() {
                    return;
                }
                let query = search.query.clone();
                match app.client.search_floors(&query, 0, app.config.search_page_size).await {
                    Ok(floors) => {
                        if let Some(Overlay::Search(s)) = &mut app.overlay {
                            s.results = floors;
                            s.has_results = true;
                            s.active = false;
                            s.list_state.select(if s.results.is_empty() {
                                None
                            } else {
                                Some(0)
                            });
                            app.status = format!("{} results", s.results.len());
                        }
                    }
                    Err(e) => app.status = format!("Search error: {}", e),
                }
            }
            KeyCode::Backspace => {
                if search.cursor > 0 {
                    let idx = search
                        .query
                        .char_indices()
                        .nth(search.cursor - 1)
                        .map(|(i, _)| i);
                    if let Some(i) = idx {
                        search.query.remove(i);
                        search.cursor -= 1;
                    }
                }
            }
            KeyCode::Left => {
                search.cursor = search.cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                if search.cursor < search.query.chars().count() {
                    search.cursor += 1;
                }
            }
            KeyCode::Char(c) => {
                let byte_idx = search
                    .query
                    .char_indices()
                    .nth(search.cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(search.query.len());
                search.query.insert(byte_idx, c);
                search.cursor += 1;
            }
            _ => {}
        }
    } else {
        match key {
            KeyCode::Esc => {
                app.overlay = None;
            }
            KeyCode::Char('/') => {
                if let Some(Overlay::Search(s)) = &mut app.overlay {
                    s.query.clear();
                    s.cursor = 0;
                    s.active = true;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = search.list_state.selected().unwrap_or(0);
                if i < search.results.len().saturating_sub(1) {
                    search.list_state.select(Some(i + 1));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = search.list_state.selected().unwrap_or(0);
                if i > 0 {
                    search.list_state.select(Some(i - 1));
                }
            }
            KeyCode::Enter => {
                let hole_id = search
                    .list_state
                    .selected()
                    .and_then(|i| search.results.get(i))
                    .map(|f| f.hole_id);
                if let Some(hid) = hole_id {
                    app.overlay = None;
                    load_hole_detail(app, hid).await;
                }
            }
            _ => {}
        }
    }
}

async fn handle_input_keys(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match key {
        KeyCode::Esc => {
            app.view = app.prev_view.take().unwrap_or(View::HoleList);
        }
        KeyCode::Enter if modifiers.contains(KeyModifiers::ALT) => {
            let byte_idx = app
                .input_buf
                .char_indices()
                .nth(app.input_cursor)
                .map(|(i, _)| i)
                .unwrap_or(app.input_buf.len());
            app.input_buf.insert(byte_idx, '\n');
            app.input_cursor += 1;
        }
        KeyCode::Enter => {
            let content = app.input_buf.clone();
            if content.trim().is_empty() {
                app.status = "Cannot submit empty content.".into();
                return;
            }
            match app.input_mode.clone() {
                InputMode::NewPost => {
                    let div_id = app.division_id();
                    let tag_names: Vec<String> = if app.input_tag_buf.is_empty() {
                        vec![]
                    } else {
                        app.input_tag_buf
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    };
                    match app.client.create_hole(div_id, &content, &tag_names).await {
                        Ok(hole) => {
                            app.status = format!("Created hole #{}", hole.id);
                            app.view = View::HoleList;
                            load_holes(app).await;
                        }
                        Err(e) => app.status = format!("Error: {}", e),
                    }
                }
                InputMode::Reply => {
                    if let Some(hole) = &app.current_hole {
                        let hid = hole.id;
                        match app.client.reply_to_hole(hid, &content).await {
                            Ok(floor) => {
                                app.status =
                                    format!("Replied as {} (floor #{})", floor.anonyname, floor.id);
                                app.view = View::HoleDetail;
                                if let Ok(floors) = app.client.get_floors(hid, 0, 50, None).await {
                                    app.floors = floors;
                                    app.floor_selected = app.floors.len().saturating_sub(1);
                                    // scroll will be auto-adjusted in render
                                    app.floor_scroll = u16::MAX;
                                }
                            }
                            Err(e) => app.status = format!("Error: {}", e),
                        }
                    }
                }
            }
        }
        KeyCode::Backspace => {
            if app.input_cursor > 0 {
                let idx = app
                    .input_buf
                    .char_indices()
                    .nth(app.input_cursor - 1)
                    .map(|(i, _)| i);
                if let Some(i) = idx {
                    app.input_buf.remove(i);
                    app.input_cursor -= 1;
                }
            }
        }
        KeyCode::Left => {
            app.input_cursor = app.input_cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            if app.input_cursor < app.input_buf.chars().count() {
                app.input_cursor += 1;
            }
        }
        KeyCode::Tab if app.input_mode == InputMode::NewPost => {
            std::mem::swap(&mut app.input_buf, &mut app.input_tag_buf);
            app.input_cursor = app.input_buf.chars().count();
        }
        KeyCode::Char(c) => {
            let byte_idx = app
                .input_buf
                .char_indices()
                .nth(app.input_cursor)
                .map(|(i, _)| i)
                .unwrap_or(app.input_buf.len());
            app.input_buf.insert(byte_idx, c);
            app.input_cursor += 1;
        }
        _ => {}
    }
}

async fn handle_message_keys(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.view = View::HoleList;
            app.messages.clear();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let i = app.message_list_state.selected().unwrap_or(0);
            if i < app.messages.len().saturating_sub(1) {
                app.message_list_state.select(Some(i + 1));
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.message_list_state.selected().unwrap_or(0);
            if i > 0 {
                app.message_list_state.select(Some(i - 1));
            }
        }
        KeyCode::Enter => {
            if let Some(i) = app.message_list_state.selected() {
                if let Some(msg) = app.messages.get(i) {
                    let mid = msg.message_id;
                    match app.client.mark_message_read(mid).await {
                        Ok(()) => {
                            app.messages[i].has_read = true;
                            app.status = format!("Marked message #{} as read", mid);
                        }
                        Err(e) => app.status = format!("Error: {}", e),
                    }
                }
            }
        }
        KeyCode::Char('c') => {
            match app.client.clear_messages().await {
                Ok(()) => {
                    app.messages.clear();
                    app.status = "All messages cleared.".into();
                }
                Err(e) => app.status = format!("Error: {}", e),
            }
        }
        KeyCode::Char(',') => open_settings(app),
        _ => {}
    }
}

// ── UI ──

fn ui(f: &mut Frame, app: &mut App) {
    let show_bar = app.config.show_help_bar;
    let constraints = if show_bar {
        vec![
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ]
    } else {
        vec![Constraint::Length(3), Constraint::Min(0)]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    render_header(f, app, chunks[0]);

    match app.view {
        View::HoleList => render_hole_list(f, app, chunks[1]),
        View::HoleDetail => render_hole_detail(f, app, chunks[1]),
        View::Input => render_input(f, app, chunks[1]),
        View::MessageList => render_messages(f, app, chunks[1]),
    }

    if show_bar {
        render_help_bar(f, app, chunks[2]);
    }

    // Overlays (on top)
    let full = f.area();
    match &app.overlay {
        Some(Overlay::Help) => render_help_overlay(f, app, full),
        Some(Overlay::DivisionPicker) => render_division_picker(f, app, full),
        Some(Overlay::Search(_)) => render_search_overlay(f, app, full),
        Some(Overlay::Settings) => render_settings_overlay(f, app, full),
        Some(Overlay::ImageViewer(_)) => render_image_viewer(f, app, full),
        None => {}
    }
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(" Danta", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" - FDU Hole (\u{6811}\u{6d1e})", Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        styled_block(&app.config)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(title, area);
}

fn render_help_bar(f: &mut Frame, app: &mut App, area: Rect) {
    let pairs = app.help_pairs();
    let mut help_spans = vec![Span::raw(" ")];
    help_spans.extend(styled_help_spans(&pairs));

    // Status on the right, separated by │
    if !app.status.is_empty() {
        let status_str = format!(" \u{2502} {} ", &app.status);
        let status_w = status_str.len() as u16;
        let help_w = area.width.saturating_sub(status_w);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(help_w), Constraint::Length(status_w)])
            .split(area);
        f.render_widget(Paragraph::new(Line::from(help_spans)), chunks[0]);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                status_str,
                Style::default().fg(Color::DarkGray),
            ))),
            chunks[1],
        );
    } else {
        f.render_widget(Paragraph::new(Line::from(help_spans)), area);
    }
}

fn render_hole_list(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .holes
        .iter()
        .map(|h| {
            let tags: Vec<_> = h.tags.iter().map(|t| t.name.as_str()).collect();
            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!("[{}] ", tags.join(", "))
            };

            let preview = h
                .floors
                .as_ref()
                .and_then(|fl| fl.first_floor.as_ref())
                .map(|fl| {
                    let s = fl.content.replace('\n', " ");
                    let max_chars = (area.width as usize).saturating_sub(20);
                    let truncated: String = s.chars().take(max_chars).collect();
                    if s.chars().count() > max_chars {
                        format!("{}...", truncated)
                    } else {
                        truncated
                    }
                })
                .unwrap_or_else(|| "...".into());

            let line1 = Line::from(vec![
                Span::styled(format!("#{} ", h.id), Style::default().fg(Color::Yellow)),
                Span::styled(tag_str, Style::default().fg(Color::Blue)),
                Span::raw(preview),
            ]);
            let line2 = Line::from(Span::styled(
                format!("  {}\u{56de}\u{590d} {}\u{6d4f}\u{89c8}  {}", h.reply, h.view, h.time_updated.get(..19).unwrap_or(&h.time_updated)),
                Style::default().fg(Color::DarkGray),
            ));
            ListItem::new(vec![line1, line2])
        })
        .collect();

    let div_name = app.division_name().to_string();
    let list = List::new(items)
        .block(
            styled_block(&app.config)
                .title(format!(" {} ", div_name))
                .title_style(Style::default().fg(Color::Cyan))
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");

    f.render_stateful_widget(list, area, &mut app.hole_list_state);
}

struct InlineImage {
    line_start: usize,
    height: u16,
    width: u16,
    url: String,
}

fn render_hole_detail(f: &mut Frame, app: &mut App, area: Rect) {
    let hole = match &app.current_hole {
        Some(h) => h,
        None => return,
    };

    let tags: Vec<_> = hole.tags.iter().map(|t| t.name.as_str()).collect();
    let loaded_marker = if app.floors_all_loaded { "" } else { "+" };
    let title = format!(
        " Hole #{} [{}] - {}\u{56de}\u{590d} ({}{} loaded) ",
        hole.id,
        tags.join(", "),
        hole.reply,
        app.floors.len(),
        loaded_marker,
    );

    let render_mode = app.config.thumbnail_mode.render_mode();
    let use_protocol = render_mode == ThumbnailRenderMode::Auto
        && app.picker.is_some();
    let content_width = area.width.saturating_sub(4) as u32;

    let mut lines: Vec<Line> = Vec::new();
    let mut inline_images: Vec<InlineImage> = Vec::new();
    let mut floor_line_starts: Vec<usize> = Vec::new();

    for (i, floor) in app.floors.iter().enumerate() {
        floor_line_starts.push(lines.len());
        let is_selected = i == app.floor_selected;
        let marker = if is_selected { "\u{203a} " } else { "  " };

        let ts = floor.time_created.get(..19).unwrap_or(&floor.time_created);

        let header = if i == 0 {
            format!("{}-- OP ({}) {} --", marker, floor.anonyname, ts)
        } else {
            format!("{}-- #{} ({}) {} --", marker, i, floor.anonyname, ts)
        };

        let header_style = if is_selected {
            Style::default().fg(Color::Green).bold()
        } else {
            Style::default().fg(Color::Yellow)
        };
        lines.push(Line::from(Span::styled(header, header_style)));

        let segments = image_parse::split_content(&floor.content);
        for seg in &segments {
            match seg {
                image_parse::ContentSegment::Text(text) => {
                    for line in text.lines() {
                        lines.push(Line::from(line.to_string()));
                    }
                }
                image_parse::ContentSegment::Image { is_sticker, url, label } => {
                    if let Some(img) = app.image_cache.get(url) {
                        let (max_w, max_h) = if *is_sticker {
                            (content_width.min(16), 4u32)
                        } else {
                            (content_width.min(60), 20)
                        };

                        if use_protocol {
                            // Protocol mode: insert blank placeholders, overlay later
                            use image::GenericImageView;
                            let (ow, oh) = img.dimensions();
                            let aspect = ow as f32 / oh as f32;
                            let h = ((max_w as f32 / aspect) / 2.0).ceil() as u16;
                            let h = h.min(max_h as u16).max(2);
                            inline_images.push(InlineImage {
                                line_start: lines.len(),
                                height: h,
                                width: max_w as u16,
                                url: url.clone(),
                            });
                            for _ in 0..h {
                                lines.push(Line::from(""));
                            }
                        } else {
                            // Half-block or ASCII
                            let img_lines = match render_mode {
                                ThumbnailRenderMode::Grayscale => {
                                    image_render::render_image_to_grayscale_lines(img, max_w, max_h)
                                }
                                ThumbnailRenderMode::Off => vec![],
                                _ => {
                                    image_render::render_image_to_colored_lines(img, max_w, max_h)
                                }
                            };
                            if img_lines.is_empty() {
                                let tag = if *is_sticker { "Sticker" } else { "Image" };
                                lines.push(Line::from(Span::styled(
                                    format!("[{}: {}]", tag, label),
                                    Style::default().fg(Color::Cyan),
                                )));
                            } else {
                                lines.extend(img_lines);
                            }
                        }
                    } else if app.image_loading.contains(url) {
                        lines.push(Line::from(Span::styled(
                            "  Loading...",
                            Style::default().fg(Color::DarkGray),
                        )));
                    } else {
                        let tag = if *is_sticker { "Sticker" } else { "Image" };
                        lines.push(Line::from(Span::styled(
                            format!("  [{}: {}]", tag, label),
                            Style::default().fg(Color::Cyan),
                        )));
                    }
                }
            }
        }

        let mut meta_parts: Vec<Span> = Vec::new();
        meta_parts.push(Span::styled("  ", Style::default()));
        if floor.like > 0 || floor.liked {
            let style = if floor.liked {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            meta_parts.push(Span::styled(format!("+{} ", floor.like), style));
        }
        if floor.dislike > 0 || floor.disliked {
            let style = if floor.disliked {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            meta_parts.push(Span::styled(format!("-{} ", floor.dislike), style));
        }
        if floor.is_me {
            meta_parts.push(Span::styled(
                "[mine] ",
                Style::default().fg(Color::Magenta),
            ));
        }
        if !floor.fold.is_empty() {
            meta_parts.push(Span::styled(
                format!("[fold: {}] ", floor.fold.join(", ")),
                Style::default().fg(Color::Red),
            ));
        }
        if meta_parts.len() > 1 {
            lines.push(Line::from(meta_parts));
        }
        lines.push(Line::from(""));
    }

    if !app.floors_all_loaded {
        lines.push(Line::from(Span::styled(
            "  \u{2193} More floors (scroll down to load) \u{2193}",
            Style::default().fg(Color::Cyan),
        )));
    }

    let block = styled_block(&app.config)
        .title(title)
        .title_style(Style::default().fg(Color::Cyan))
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);

    // Auto-adjust scroll to keep selected floor visible
    if let Some(&sel_start) = floor_line_starts.get(app.floor_selected) {
        let sel_end = floor_line_starts
            .get(app.floor_selected + 1)
            .copied()
            .unwrap_or(lines.len());
        let visible = inner.height as usize;
        let scroll = app.floor_scroll as usize;

        if sel_start < scroll {
            // Selected floor is above viewport — scroll up to it
            app.floor_scroll = sel_start as u16;
        } else if sel_end > scroll + visible {
            // Selected floor's bottom is below viewport — scroll down
            app.floor_scroll = sel_end.saturating_sub(visible) as u16;
        }
    }

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((app.floor_scroll, 0));
    f.render_widget(paragraph, area);

    // Phase 2: overlay protocol images on top of blank placeholders
    if use_protocol && !inline_images.is_empty() {
        let scroll = app.floor_scroll as usize;
        let visible = inner.height as usize;

        for img_info in &inline_images {
            let top = img_info.line_start;
            let h = img_info.height as usize;
            // Visibility check
            if top + h <= scroll || top >= scroll + visible {
                continue;
            }
            let y_in_view = top.saturating_sub(scroll);
            let clip_top = scroll.saturating_sub(top);
            let avail = (visible - y_in_view).min(h - clip_top);
            if avail == 0 {
                continue;
            }

            let rect = Rect::new(
                inner.x,
                inner.y + y_in_view as u16,
                img_info.width.min(inner.width),
                avail as u16,
            );

            // Per-occurrence cache key to prevent shared protocol thrashing
            let cache_key = format!("{}@{}", img_info.url, img_info.line_start);
            if app.image_cache.contains_key(&img_info.url) {
                if let Some(picker) = &app.picker {
                    if !app.protocol_cache.contains_key(&cache_key) {
                        let img = app.image_cache[&img_info.url].clone();
                        let proto = picker.new_resize_protocol(img);
                        app.protocol_cache.insert(cache_key.clone(), proto);
                    }
                }
                if let Some(proto) = app.protocol_cache.get_mut(&cache_key) {
                    f.render_stateful_widget(
                        ratatui_image::StatefulImage::default(),
                        rect,
                        proto,
                    );
                }
            }
        }
    }
}

fn render_search_overlay(f: &mut Frame, app: &mut App, area: Rect) {
    let overlay_area = centered_rect(75, 65, area);
    f.render_widget(Clear, overlay_area);

    let search = match &mut app.overlay {
        Some(Overlay::Search(s)) => s,
        _ => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(overlay_area);

    // Search input
    let input_style = if search.active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let input = Paragraph::new(search.query.as_str()).block(
        styled_block(&app.config)
            .title(Span::styled(
                " Search ",
                Style::default().fg(Color::Cyan),
            ))
            .border_style(input_style),
    );
    f.render_widget(input, chunks[0]);

    if search.active {
        let cx = chunks[0].x + 1 + search.cursor as u16;
        let cy = chunks[0].y + 1;
        f.set_cursor_position((cx.min(chunks[0].right() - 2), cy));
    }

    // Results
    if search.has_results {
        let items: Vec<ListItem> = search
            .results
            .iter()
            .map(|fl| {
                let preview: String = fl.content.replace('\n', " ").chars().take(60).collect();
                let line = Line::from(vec![
                    Span::styled(
                        format!("#{} ", fl.hole_id),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(
                        format!("({}) ", fl.anonyname),
                        Style::default().fg(Color::Blue),
                    ),
                    Span::raw(preview),
                ]);
                ListItem::new(line)
            })
            .collect();

        let results_title = format!(" {} results ", search.results.len());
        let list = List::new(items)
            .block(
                styled_block(&app.config)
                    .title(Span::styled(results_title, Style::default().fg(Color::Cyan)))
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("\u{203a} ");

        f.render_stateful_widget(list, chunks[1], &mut search.list_state);
    } else {
        let hint = hint_line(&[("Enter", "search"), ("Esc", "close")]);
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Type a query and press Enter",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            hint,
        ])
        .block(
            styled_block(&app.config)
                .title(Span::styled(" Results ", Style::default().fg(Color::Cyan)))
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(empty, chunks[1]);
    }
}

fn render_help_overlay(f: &mut Frame, app: &App, area: Rect) {
    // Exact same sizing as pikpaktui
    let sheet_w = area.width.saturating_sub(4).min(92).max(44);
    let inner_w = sheet_w.saturating_sub(2) as usize;
    let show_art = inner_w >= 70 && app.config.show_ascii_art;

    type HelpSection = (&'static str, Vec<(&'static str, &'static str)>);

    let sections: Vec<HelpSection> = vec![
        (
            "Navigation",
            vec![
                ("j / \u{2193}", "Down"),
                ("k / \u{2191}", "Up"),
                ("g / G", "Top / end"),
                ("Enter", "Open"),
                ("Bksp", "Back"),
                ("PgDn/Up", "Page"),
            ],
        ),
        (
            "Actions",
            vec![
                ("n", "New post"),
                ("r", "Reply"),
                ("i", "View image"),
                ("l / d", "Like/dislike"),
                ("f / F", "Fav/unfav"),
                ("s / S", "Sub/unsub"),
                ("/", "Search"),
            ],
        ),
        (
            "Panels",
            vec![
                ("Tab", "Division"),
                ("m", "Messages"),
                (",", "Settings"),
                ("h", "Help"),
                ("q / Esc", "Quit/back"),
                ("Ctrl+C", "Force quit"),
            ],
        ),
    ];

    let key_w: usize = 7;
    let columns: Vec<Vec<(&str, &Vec<(&str, &str)>)>> = sections
        .iter()
        .map(|(name, items)| vec![(*name, items)])
        .collect();
    let col_count = columns.len();
    let col_w = inner_w / col_count;

    let col_heights: Vec<usize> = columns
        .iter()
        .map(|groups| {
            groups.iter().enumerate().fold(0, |h, (i, (_, items))| {
                h + if i > 0 { 1 } else { 0 } + 1 + items.len()
            })
        })
        .collect();
    let max_rows = col_heights.iter().copied().max().unwrap_or(0);

    let min_content_h = max_rows + 2 + 2;
    let art_lines: usize = 7;
    let show_art = show_art && (area.height as usize) >= min_content_h + art_lines;
    let art_h: usize = if show_art { art_lines } else { 1 };
    let content_h = art_h + max_rows + 2;
    let sheet_h = ((content_h + 2) as u16).min(area.height);

    let x = (area.width.saturating_sub(sheet_w)) / 2;
    let y = (area.height.saturating_sub(sheet_h)) / 2;
    let sheet_area = Rect::new(x, y, sheet_w, sheet_h);

    clear_overlay_area(f, sheet_area);

    let title_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let key_style = Style::default().fg(Color::Yellow);
    let desc_style = Style::default().fg(Color::Reset);

    let mut lines: Vec<Line> = Vec::new();

    if show_art {
        lines.push(Line::from(""));
        for (text, &color) in LOGO.iter().zip(LOGO_COLORS.iter()) {
            let art_w = text.chars().count();
            let pad = inner_w.saturating_sub(art_w) / 2;
            lines.push(Line::from(Span::styled(
                format!("{}{}", " ".repeat(pad), text),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )));
        }
        lines.push(Line::from(""));
    } else {
        lines.push(Line::from(""));
    }

    enum RowKind { Title(&'static str), Item(&'static str, &'static str), Blank }
    let col_rows: Vec<Vec<RowKind>> = columns
        .iter()
        .map(|groups| {
            let mut rows = Vec::new();
            for (i, (name, items)) in groups.iter().enumerate() {
                if i > 0 { rows.push(RowKind::Blank); }
                rows.push(RowKind::Title(name));
                for &(key, desc) in *items { rows.push(RowKind::Item(key, desc)); }
            }
            rows
        })
        .collect();

    // Render rows side by side — exact pikpaktui pattern
    for row in 0..max_rows {
        let mut spans = Vec::new();
        for (ci, rows) in col_rows.iter().enumerate() {
            let prefix = if ci == 0 { " " } else { "" };
            if row < rows.len() {
                match &rows[row] {
                    RowKind::Title(name) => {
                        let w = col_w.saturating_sub(prefix.len());
                        spans.push(Span::styled(
                            format!("{}{:<width$}", prefix, name, width = w),
                            title_style,
                        ));
                    }
                    RowKind::Item(key, desc) => {
                        let dw = col_w.saturating_sub(key_w + 1 + prefix.len());
                        spans.push(Span::styled(
                            format!("{}{:<kw$} ", prefix, key, kw = key_w),
                            key_style,
                        ));
                        spans.push(Span::styled(
                            format!("{:<dw$}", desc, dw = dw),
                            desc_style,
                        ));
                    }
                    RowKind::Blank => {
                        spans.push(Span::raw(format!("{:<width$}", "", width = col_w)));
                    }
                }
            } else {
                spans.push(Span::raw(format!("{:<width$}", "", width = col_w)));
            }
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Press any key to close",
        Style::default().fg(Color::DarkGray),
    )));

    let help = Paragraph::new(Text::from(lines)).block(
        styled_block(&app.config)
            .title(Span::styled(
                " Help ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(help, sheet_area);
}

fn render_settings_overlay(f: &mut Frame, app: &App, area: Rect) {
    let overlay_area = centered_rect(65, 60, area);
    clear_overlay_area(f, overlay_area);

    let s = &app.settings;

    type Item = (&'static str, &'static str, String);
    let items: Vec<Item> = vec![
        ("Border Style", "Window border appearance", s.draft.border_style.as_str().to_string()),
        ("Show Help Bar", "Display keybindings at bottom", if s.draft.show_help_bar { "[\u{2713}]".into() } else { "[ ]".into() }),
        ("Show ASCII Art", "Logo in help overlay", if s.draft.show_ascii_art { "[\u{2713}]".into() } else { "[ ]".into() }),
        ("Sort Order", "Hole listing order", s.draft.sort_order.display().to_string()),
        ("Floors / Page", "Floors loaded per batch", format!("{}", s.draft.floors_per_page)),
        ("Search Results", "Results per search", format!("{}", s.draft.search_page_size)),
        ("Default Division", "Starting division ID", format!("{}", s.draft.default_division)),
        ("Image Mode", "How images are rendered", s.draft.thumbnail_mode.display_name().to_string()),
        ("Image Protocol", "Terminal image protocol", s.draft.image_protocol.display_name().to_string()),
    ];

    let mut lines: Vec<Line> = vec![Line::from("")];
    lines.push(Line::from(Span::styled(
        " Settings",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));

    for (i, (name, desc, value)) in items.iter().enumerate() {
        let is_sel = i == s.selected;
        let prefix = if is_sel { " \u{203a} " } else { "   " };

        let (name_style, val_style) = if is_sel && s.editing {
            (
                Style::default().fg(Color::Yellow).bold(),
                Style::default().fg(Color::Yellow).bold(),
            )
        } else if is_sel {
            (
                Style::default().fg(Color::Cyan).bold(),
                Style::default().fg(Color::Green),
            )
        } else {
            (
                Style::default().fg(Color::White),
                Style::default().fg(Color::Green),
            )
        };

        let inner = overlay_area.width.saturating_sub(2) as usize;
        let name_w = prefix.chars().count() + name.len();
        let pad = inner.saturating_sub(name_w + value.len() + 1);

        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), name_style),
            Span::styled(name.to_string(), name_style),
            Span::raw(" ".repeat(pad)),
            Span::styled(value.clone(), val_style),
        ]));

        lines.push(Line::from(Span::styled(
            format!("     {}", desc),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    let hints: &[(&str, &str)] = if s.editing {
        &[("\u{2190}/\u{2192}", "change"), ("Esc", "done")]
    } else {
        &[("j/k", "nav"), ("Space", "edit"), ("s", "save"), ("Esc", "close")]
    };
    lines.push(hint_line(hints));

    let title = if s.modified { " Settings * " } else { " Settings " };
    let p = Paragraph::new(Text::from(lines)).block(
        styled_block(&app.config)
            .title(Span::styled(
                title,
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(p, overlay_area);
}

fn render_image_viewer(f: &mut Frame, app: &mut App, area: Rect) {
    let overlay_area = centered_rect(85, 80, area);
    clear_overlay_area(f, overlay_area);

    let state = match &app.overlay {
        Some(Overlay::ImageViewer(s)) => s,
        _ => return,
    };

    let (alt, url) = &state.urls[state.current_index];
    let counter = if state.urls.len() > 1 {
        format!(" {}/{} ", state.current_index + 1, state.urls.len())
    } else {
        String::new()
    };
    let title = if alt.is_empty() {
        format!(" Image{} ", counter)
    } else {
        format!(" {}{} ", alt, counter)
    };

    let inner = Rect {
        x: overlay_area.x + 1,
        y: overlay_area.y + 1,
        width: overlay_area.width.saturating_sub(2),
        height: overlay_area.height.saturating_sub(2),
    };

    if let Some(img) = app.image_cache.get(url) {
        let render_mode = app.config.thumbnail_mode.render_mode();

        // Reserve 2 lines for hints at bottom
        let image_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        };
        let hint_area = Rect {
            x: inner.x,
            y: inner.y + image_area.height,
            width: inner.width,
            height: 2.min(inner.height),
        };

        match render_mode {
            ThumbnailRenderMode::Auto => {
                if let Some(picker) = &app.picker {
                    let viewer_key = format!("__viewer__{}", url);
                    if !app.protocol_cache.contains_key(&viewer_key) {
                        let proto = picker.new_resize_protocol(img.clone());
                        app.protocol_cache.insert(viewer_key.clone(), proto);
                    }
                    if let Some(proto) = app.protocol_cache.get_mut(&viewer_key) {
                        f.render_stateful_widget(
                            ratatui_image::StatefulImage::default(),
                            image_area,
                            proto,
                        );
                    }
                }
            }
            ThumbnailRenderMode::ColoredHalf => {
                let colored_lines = image_render::render_image_to_colored_lines(
                    img,
                    image_area.width as u32,
                    image_area.height as u32,
                );
                f.render_widget(Paragraph::new(Text::from(colored_lines)), image_area);
            }
            ThumbnailRenderMode::Grayscale => {
                let ascii_lines = image_render::render_image_to_grayscale_lines(
                    img,
                    image_area.width as u32,
                    image_area.height as u32,
                );
                f.render_widget(
                    Paragraph::new(Text::from(ascii_lines)).style(Style::default().fg(Color::DarkGray)),
                    image_area,
                );
            }
            ThumbnailRenderMode::Off => {
                f.render_widget(
                    Paragraph::new(Span::styled("  Image display disabled", Style::default().fg(Color::DarkGray))),
                    image_area,
                );
            }
        }

        // Hints
        let hints: &[(&str, &str)] = if state.urls.len() > 1 {
            &[("h/l", "prev/next"), ("Esc", "close")]
        } else {
            &[("Esc", "close")]
        };
        f.render_widget(Paragraph::new(hint_line(hints)), hint_area);
    } else if app.image_loading.contains(url) {
        let loading = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  Loading image...", Style::default().fg(Color::Cyan))),
        ]);
        f.render_widget(loading, inner);
    } else {
        let failed = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  Failed to load image", Style::default().fg(Color::Red))),
        ]);
        f.render_widget(failed, inner);
    }

    // Border
    let border = styled_block(&app.config)
        .title(Span::styled(title, Style::default().fg(Color::Cyan).bold()))
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(border, overlay_area);
}

fn render_division_picker(f: &mut Frame, app: &mut App, area: Rect) {
    let count = app.divisions.len().max(1) as u16;
    let h = (count * 2 + 4).min(area.height.saturating_sub(6));
    let w = 50u16.min(area.width.saturating_sub(10));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay_area = Rect::new(x, y, w, h);

    f.render_widget(Clear, overlay_area);

    let items: Vec<ListItem> = app
        .divisions
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let marker = if i == app.current_division {
                "\u{2605} "
            } else {
                "  "
            };
            let line = Line::from(vec![
                Span::styled(
                    marker.to_string(),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    d.name.clone(),
                    Style::default().fg(Color::White).bold(),
                ),
                Span::styled(
                    format!("  {}", d.description),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            styled_block(&app.config)
                .title(Span::styled(
                    " Switch Division ",
                    Style::default().fg(Color::Cyan),
                ))
                .border_style(Style::default().fg(Color::LightMagenta)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");

    f.render_stateful_widget(list, overlay_area, &mut app.division_picker_state);
}

fn render_input(f: &mut Frame, app: &mut App, area: Rect) {
    let title = match app.input_mode {
        InputMode::NewPost => " New Post ",
        InputMode::Reply => " Reply ",
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if app.input_mode == InputMode::NewPost {
            vec![Constraint::Length(3), Constraint::Min(5)]
        } else {
            vec![Constraint::Length(0), Constraint::Min(5)]
        })
        .split(area);

    if app.input_mode == InputMode::NewPost {
        let tag_display = if app.input_tag_buf.is_empty() {
            "Tags: (none - press Tab to add)".to_string()
        } else {
            format!("Tags: {}", app.input_tag_buf)
        };
        let tag_para = Paragraph::new(tag_display)
            .block(
                styled_block(&app.config)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .style(Style::default().fg(Color::Blue));
        f.render_widget(tag_para, chunks[0]);
    }

    let input_para = Paragraph::new(app.input_buf.as_str())
        .block(
            styled_block(&app.config)
                .title(Span::styled(title, Style::default().fg(Color::Cyan)))
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(input_para, chunks[1]);

    let inner_width = chunks[1].width.saturating_sub(2).max(1) as usize;
    let cursor_x = chunks[1].x + 1 + (app.input_cursor % inner_width) as u16;
    let cursor_y = chunks[1].y + 1 + (app.input_cursor / inner_width) as u16;
    f.set_cursor_position((cursor_x, cursor_y));
}

fn render_messages(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .messages
        .iter()
        .map(|m| {
            let read_marker = if m.has_read { " " } else { "\u{2022}" };
            let ts = m.time_created.get(..19).unwrap_or(&m.time_created);
            let line1 = Line::from(vec![
                Span::styled(
                    format!("{} ", read_marker),
                    if m.has_read {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::Yellow).bold()
                    },
                ),
                Span::styled(format!("[{}] ", ts), Style::default().fg(Color::DarkGray)),
                Span::raw(&m.description),
            ]);
            let mut lines = vec![line1];
            if !m.message.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", m.message),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            ListItem::new(lines)
        })
        .collect();

    let list = List::new(items)
        .block(
            styled_block(&app.config)
                .title(Span::styled(
                    format!(" Messages ({}) ", app.messages.len()),
                    Style::default().fg(Color::Cyan),
                ))
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{203a} ");

    f.render_stateful_widget(list, area, &mut app.message_list_state);
}
