mod api;
mod skill;

use api::AnthropicClient;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::io;
use std::time::Duration;
use tokio::sync::oneshot;

const MAX_QUESTIONS: usize = 500;
const MIN_QUESTIONS: usize = 5;

// ─── Theme ─────────────────────────────────────────────────────

const ACCENT: Color = Color::Rgb(6, 182, 212);
const DIM: Color = Color::Rgb(100, 116, 139);
const SURFACE: Color = Color::Rgb(15, 23, 42);
const SURFACE_LIGHT: Color = Color::Rgb(30, 41, 59);
const TEXT: Color = Color::Rgb(226, 232, 240);
const GREEN: Color = Color::Rgb(34, 197, 94);
const RED: Color = Color::Rgb(239, 68, 68);
const BORDER: Color = Color::Rgb(51, 65, 85);

fn lerp(a: f64, b: f64, t: f64) -> u8 {
    (a + (b - a) * t) as u8
}

fn gradient_color(t: f64) -> Color {
    // Blue → Cyan → Teal
    let (r, g, b) = if t < 0.5 {
        let t = t * 2.0;
        (lerp(59.0, 6.0, t), lerp(130.0, 182.0, t), lerp(246.0, 212.0, t))
    } else {
        let t = (t - 0.5) * 2.0;
        (lerp(6.0, 20.0, t), lerp(182.0, 184.0, t), lerp(212.0, 166.0, t))
    };
    Color::Rgb(r, g, b)
}

fn title_spans() -> Vec<Span<'static>> {
    let title = "write-like-me";
    let mut spans = vec![Span::styled("◆ ", Style::default().fg(ACCENT))];
    let len = title.len().max(1) - 1;
    for (i, c) in title.chars().enumerate() {
        let t = if len > 0 { i as f64 / len as f64 } else { 0.0 };
        spans.push(Span::styled(
            c.to_string(),
            Style::default().fg(gradient_color(t)).bold(),
        ));
    }
    spans
}

fn thinking_dots(tick: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for i in 0..5 {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let phase = (tick as f64 * 0.3 + i as f64 * 0.8).sin();
        let brightness = ((phase + 1.0) / 2.0 * 200.0 + 55.0) as u8;
        spans.push(Span::styled(
            "●",
            Style::default().fg(Color::Rgb(
                (brightness as f64 * 0.02) as u8,
                (brightness as f64 * 0.71) as u8,
                (brightness as f64 * 0.83) as u8,
            )),
        ));
    }
    spans
}

fn pulsing_border(tick: usize) -> Color {
    let t = ((tick as f64 * 0.12).sin() + 1.0) / 2.0;
    gradient_color(t)
}

fn pulsing_accent(tick: usize) -> Color {
    let brightness = ((tick as f64 * 0.15).sin() + 1.0) / 2.0 * 180.0 + 75.0;
    Color::Rgb(
        (brightness * 0.02) as u8,
        (brightness * 0.71) as u8,
        (brightness * 0.83) as u8,
    )
}

// ─── App ───────────────────────────────────────────────────────

enum Phase {
    Welcome,
    Generating,
    Answering,
    Analyzing,
    Results {
        profile_path: String,
        skill_path: String,
    },
    Error(String),
}

enum PendingResult {
    Question(Result<String, String>),
    Profile(Result<String, String>),
}

struct App {
    phase: Phase,
    client: AnthropicClient,
    samples: Vec<(String, String)>,
    current_question: String,
    input: String,
    cursor: usize,
    question_num: usize,
    tick: usize,
    reveal: usize,
    pending_rx: Option<oneshot::Receiver<PendingResult>>,
    should_quit: bool,
}

impl App {
    fn new(client: AnthropicClient) -> Self {
        Self {
            phase: Phase::Welcome,
            client,
            samples: Vec::new(),
            current_question: String::new(),
            input: String::new(),
            cursor: 0,
            question_num: 0,
            tick: 0,
            reveal: 0,
            pending_rx: None,
            should_quit: false,
        }
    }

    fn start_generating(&mut self) {
        self.phase = Phase::Generating;
        self.question_num += 1;
        let client = self.client.clone();
        let samples = self.samples.clone();
        let qnum = self.question_num;
        let (tx, rx) = oneshot::channel();
        self.pending_rx = Some(rx);
        tokio::spawn(async move {
            let result = client.generate_question(&samples, qnum).await;
            let _ = tx.send(PendingResult::Question(result));
        });
    }

    fn start_analyzing(&mut self) {
        self.phase = Phase::Analyzing;
        let client = self.client.clone();
        let samples = self.samples.clone();
        let (tx, rx) = oneshot::channel();
        self.pending_rx = Some(rx);
        tokio::spawn(async move {
            let result = client.analyze_style(&samples).await;
            let _ = tx.send(PendingResult::Profile(result));
        });
    }

    fn check_pending(&mut self) {
        if let Some(ref mut rx) = self.pending_rx {
            match rx.try_recv() {
                Ok(PendingResult::Question(Ok(q))) => {
                    self.current_question = q;
                    self.input.clear();
                    self.cursor = 0;
                    self.reveal = 0;
                    self.phase = Phase::Answering;
                    self.pending_rx = None;
                }
                Ok(PendingResult::Question(Err(e))) => {
                    self.pending_rx = None;
                    if self.question_num == 1 {
                        self.phase = Phase::Error(e);
                    } else if self.samples.len() >= MIN_QUESTIONS {
                        self.start_analyzing();
                    } else {
                        self.phase = Phase::Error(format!(
                            "API error with only {} samples (need {MIN_QUESTIONS}): {e}",
                            self.samples.len()
                        ));
                    }
                }
                Ok(PendingResult::Profile(Ok(profile))) => {
                    self.pending_rx = None;
                    self.finish_with_profile(profile);
                }
                Ok(PendingResult::Profile(Err(e))) => {
                    self.pending_rx = None;
                    self.phase = Phase::Error(e);
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending_rx = None;
                    self.phase = Phase::Error("API task was cancelled".to_string());
                }
            }
        }
    }

    fn finish_with_profile(&mut self, style_profile: String) {
        let data_dir = "write-like-me-output";
        std::fs::create_dir_all(data_dir).ok();
        let profile_path = format!("{data_dir}/style-profile.md");
        std::fs::write(&profile_path, &style_profile).ok();

        match skill::generate_skill(&style_profile, data_dir) {
            Ok(skill_path) => {
                let samples_json: Vec<serde_json::Value> = self
                    .samples
                    .iter()
                    .map(|(q, a)| serde_json::json!({"question": q, "answer": a}))
                    .collect();
                std::fs::write(
                    format!("{data_dir}/samples.json"),
                    serde_json::to_string_pretty(&samples_json).unwrap_or_default(),
                )
                .ok();

                self.phase = Phase::Results {
                    profile_path,
                    skill_path,
                };
            }
            Err(e) => self.phase = Phase::Error(e),
        }
    }

    fn submit_answer(&mut self) {
        let answer = self.input.trim().to_string();
        if answer.is_empty() {
            return;
        }
        self.samples
            .push((self.current_question.clone(), answer));
        if self.question_num >= MAX_QUESTIONS {
            self.start_analyzing();
        } else {
            self.start_generating();
        }
    }

    fn finish_early(&mut self) {
        if self.samples.len() >= MIN_QUESTIONS {
            self.start_analyzing();
        }
    }
}

// ─── Input handling ────────────────────────────────────────────

fn char_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn handle_event(app: &mut App, ev: Event) {
    let Event::Key(key) = ev else { return };
    if key.kind != KeyEventKind::Press {
        return;
    }

    // Global quit
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    match &app.phase {
        Phase::Welcome => {
            if matches!(key.code, KeyCode::Enter) {
                app.start_generating();
            }
        }
        Phase::Answering => {
            // Skip typewriter reveal on any key
            let q_len = app.current_question.chars().count();
            if app.reveal < q_len {
                app.reveal = q_len;
                return;
            }

            match key.code {
                KeyCode::Enter => app.submit_answer(),
                KeyCode::Esc => app.finish_early(),
                KeyCode::Backspace => {
                    if app.cursor > 0 {
                        let idx = char_byte_index(&app.input, app.cursor - 1);
                        let end = char_byte_index(&app.input, app.cursor);
                        app.input.drain(idx..end);
                        app.cursor -= 1;
                    }
                }
                KeyCode::Left => {
                    app.cursor = app.cursor.saturating_sub(1);
                }
                KeyCode::Right => {
                    let max = app.input.chars().count();
                    app.cursor = (app.cursor + 1).min(max);
                }
                KeyCode::Home => app.cursor = 0,
                KeyCode::End => app.cursor = app.input.chars().count(),
                KeyCode::Char(c) => {
                    let idx = char_byte_index(&app.input, app.cursor);
                    app.input.insert(idx, c);
                    app.cursor += 1;
                }
                _ => {}
            }
        }
        Phase::Results { .. } | Phase::Error(_) => {
            if matches!(
                key.code,
                KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc
            ) {
                app.should_quit = true;
            }
        }
        _ => {}
    }
}

// ─── Rendering ─────────────────────────────────────────────────

fn centered_rect(max_width: u16, area: Rect) -> Rect {
    let width = max_width.min(area.width);
    let margin_y = (area.height / 8).max(1);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let height = area.height.saturating_sub(margin_y * 2);
    Rect::new(x, area.y + margin_y, width, height)
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let right_text = match &app.phase {
        Phase::Results { .. } => Span::styled("✓ complete", Style::default().fg(GREEN).bold()),
        _ if app.samples.is_empty() => Span::raw(""),
        _ => {
            let n = app.samples.len();
            let color = if n >= MIN_QUESTIONS { GREEN } else { DIM };
            Span::styled(format!("{n} samples"), Style::default().fg(color))
        }
    };

    let cols =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(12)]).split(area);

    f.render_widget(Paragraph::new(Line::from(title_spans())), cols[0]);
    f.render_widget(
        Paragraph::new(Line::from(right_text)).alignment(Alignment::Right),
        cols[1],
    );
}

fn render_separator(f: &mut Frame, area: Rect) {
    let width = area.width as usize;
    f.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(width),
            Style::default().fg(BORDER),
        )),
        area,
    );
}

fn render_welcome(f: &mut Frame, area: Rect, tick: usize) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(1),
    ])
    .split(area);

    let lines = [
        ("I'll ask you questions to learn how you write.", TEXT),
        ("Answer naturally — type the way you actually type.", TEXT),
        ("", TEXT),
        (
            &format!("minimum {MIN_QUESTIONS} responses to build your profile"),
            DIM,
        ),
    ];

    for (i, (text, color)) in lines.iter().enumerate() {
        f.render_widget(
            Paragraph::new(Span::styled(*text, Style::default().fg(*color)))
                .alignment(Alignment::Center),
            chunks[i],
        );
    }

    let color = pulsing_accent(tick);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▸ ", Style::default().fg(color)),
            Span::styled("press enter to begin", Style::default().fg(color)),
        ]))
        .alignment(Alignment::Center),
        chunks[6],
    );
}

fn render_generating(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Min(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(2),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(thinking_dots(app.tick))).alignment(Alignment::Center),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            format!("generating question {}...", app.question_num),
            Style::default().fg(DIM),
        ))
        .alignment(Alignment::Center),
        chunks[3],
    );
}

fn render_answering(f: &mut Frame, area: Rect, app: &App) {
    // Question number
    let q_num_line = Line::from(vec![
        Span::styled(
            format!("question {}", app.question_num),
            Style::default().fg(DIM),
        ),
    ]);

    // Revealed question text
    let revealed: String = app.current_question.chars().take(app.reveal).collect();
    let q_len = app.current_question.chars().count();
    let reveal_done = app.reveal >= q_len;

    // Calculate question height (rough: chars / width + 1)
    let q_height = ((q_len as u16) / area.width.max(1) + 2).min(6);

    let chunks = Layout::vertical([
        Constraint::Length(1),  // question number
        Constraint::Length(1),  // spacer
        Constraint::Length(q_height), // question text
        Constraint::Length(1),  // spacer
        Constraint::Length(3),  // input box
        Constraint::Min(0),     // flex
        Constraint::Length(1),  // footer
    ])
    .split(area);

    f.render_widget(Paragraph::new(q_num_line), chunks[0]);

    // Question text with cursor blink at end during reveal
    let mut q_spans = vec![Span::styled(
        &revealed,
        Style::default().fg(TEXT).bold(),
    )];
    if !reveal_done {
        if app.tick % 4 < 2 {
            q_spans.push(Span::styled("▌", Style::default().fg(ACCENT)));
        }
    }
    f.render_widget(
        Paragraph::new(Line::from(q_spans)).wrap(Wrap { trim: false }),
        chunks[2],
    );

    // Input box
    let input_border_color = if reveal_done { ACCENT } else { BORDER };
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(input_border_color))
        .style(Style::default().bg(SURFACE_LIGHT));
    let input_inner = input_block.inner(chunks[4]);
    f.render_widget(input_block, chunks[4]);

    if reveal_done {
        if app.input.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "type your response...",
                    Style::default().fg(Color::Rgb(71, 85, 105)),
                )),
                input_inner,
            );
        }

        // Render input with horizontal scrolling
        let input_chars: Vec<char> = app.input.chars().collect();
        let width = input_inner.width as usize;
        let (display_start, visual_cursor) = if app.cursor < width {
            (0, app.cursor)
        } else {
            (app.cursor - width + 1, width - 1)
        };
        let display_end = (display_start + width).min(input_chars.len());
        if !input_chars.is_empty() {
            let display: String = input_chars
                [display_start..display_end]
                .iter()
                .collect();
            f.render_widget(
                Paragraph::new(Span::styled(display, Style::default().fg(TEXT))),
                input_inner,
            );
        }

        f.set_cursor_position((
            input_inner.x + visual_cursor as u16,
            input_inner.y,
        ));
    }

    // Footer
    let min_met = app.samples.len() >= MIN_QUESTIONS;
    let mut footer_spans = vec![
        Span::styled("enter", Style::default().fg(TEXT).bold()),
        Span::styled(" submit", Style::default().fg(DIM)),
    ];
    if min_met {
        footer_spans.extend([
            Span::styled("  ·  ", Style::default().fg(BORDER)),
            Span::styled("esc", Style::default().fg(TEXT).bold()),
            Span::styled(" finish", Style::default().fg(DIM)),
        ]);
    } else {
        footer_spans.extend([
            Span::styled("  ·  ", Style::default().fg(BORDER)),
            Span::styled(
                format!("{} more needed", MIN_QUESTIONS - app.samples.len()),
                Style::default().fg(Color::Rgb(234, 179, 8)),
            ),
        ]);
    }
    f.render_widget(
        Paragraph::new(Line::from(footer_spans)).alignment(Alignment::Center),
        chunks[6],
    );
}

fn render_analyzing(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Min(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(2),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(thinking_dots(app.tick))).alignment(Alignment::Center),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            format!("analyzing {} samples...", app.samples.len()),
            Style::default().fg(DIM),
        ))
        .alignment(Alignment::Center),
        chunks[3],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            "building your complete style profile",
            Style::default().fg(Color::Rgb(71, 85, 105)),
        ))
        .alignment(Alignment::Center),
        chunks[4],
    );
}

fn render_results(f: &mut Frame, area: Rect, profile_path: &str, skill_path: &str) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // heading
        Constraint::Length(1), // spacer
        Constraint::Length(1), // files label
        Constraint::Length(1), // profile
        Constraint::Length(1), // skill
        Constraint::Length(1), // samples
        Constraint::Length(2), // spacer
        Constraint::Length(1), // next label
        Constraint::Length(1), // cp command
        Constraint::Length(2), // spacer
        Constraint::Length(1), // usage hint
        Constraint::Min(1),   // flex
        Constraint::Length(1), // exit hint
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Span::styled(
            "your writing style has been captured.",
            Style::default().fg(GREEN).bold(),
        ))
        .alignment(Alignment::Center),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            "files created:",
            Style::default().fg(TEXT).bold(),
        )),
        chunks[2],
    );

    let file_lines: [(& str, &str); 3] = [
        ("◇ style profile  ", profile_path),
        ("◇ claude skill    ", skill_path),
        ("◇ raw samples     ", "write-like-me-output/samples.json"),
    ];
    for (i, (label, path)) in file_lines.iter().enumerate() {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {label}"), Style::default().fg(ACCENT)),
                Span::styled(*path, Style::default().fg(DIM)),
            ])),
            chunks[3 + i],
        );
    }

    f.render_widget(
        Paragraph::new(Span::styled(
            "next — copy the skill into your project:",
            Style::default().fg(TEXT),
        )),
        chunks[7],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            "  cp -r write-like-me-output/.claude /path/to/your/project/",
            Style::default().fg(ACCENT),
        )),
        chunks[8],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("then ask Claude Code to ", Style::default().fg(DIM)),
            Span::styled("\"write like me\"", Style::default().fg(TEXT).bold()),
        ]))
        .alignment(Alignment::Center),
        chunks[10],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            "press any key to exit",
            Style::default().fg(Color::Rgb(71, 85, 105)),
        ))
        .alignment(Alignment::Center),
        chunks[12],
    );
}

fn render_error(f: &mut Frame, area: Rect, msg: &str) {
    let chunks = Layout::vertical([
        Constraint::Min(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(2),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Span::styled(
            "something went wrong",
            Style::default().fg(RED).bold(),
        ))
        .alignment(Alignment::Center),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(Span::styled(msg, Style::default().fg(DIM)))
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Center),
        chunks[3],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            "press any key to exit",
            Style::default().fg(Color::Rgb(71, 85, 105)),
        ))
        .alignment(Alignment::Center),
        chunks[5],
    );
}

fn ui(f: &mut Frame, app: &App) {
    // Dark background
    f.render_widget(Block::default().style(Style::default().bg(SURFACE)), f.area());

    let content = centered_rect(72, f.area());

    // Outer border — pulses during AI thinking
    let border_color = match app.phase {
        Phase::Generating | Phase::Analyzing => pulsing_border(app.tick),
        _ => BORDER,
    };
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .padding(Padding::new(2, 2, 1, 1));

    let inner = outer.inner(content);
    f.render_widget(outer, content);

    // Header + separator + content area
    let layout = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // separator
        Constraint::Length(1), // spacer
        Constraint::Min(4),   // content
    ])
    .split(inner);

    render_header(f, layout[0], app);
    render_separator(f, layout[1]);

    match &app.phase {
        Phase::Welcome => render_welcome(f, layout[3], app.tick),
        Phase::Generating => render_generating(f, layout[3], app),
        Phase::Answering => render_answering(f, layout[3], app),
        Phase::Analyzing => render_analyzing(f, layout[3], app),
        Phase::Results {
            profile_path,
            skill_path,
        } => render_results(f, layout[3], profile_path, skill_path),
        Phase::Error(msg) => render_error(f, layout[3], msg),
    }
}

// ─── Terminal setup ────────────────────────────────────────────

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
}

// ─── Main ──────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            eprintln!(
                "error: ANTHROPIC_API_KEY is not set.\nRun: export ANTHROPIC_API_KEY=sk-ant-..."
            );
            std::process::exit(1);
        }
    };

    // Restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    let mut terminal = match setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to initialize terminal: {e}");
            std::process::exit(1);
        }
    };

    let client = AnthropicClient::new(api_key);
    let mut app = App::new(client);

    loop {
        app.check_pending();

        // Advance typewriter reveal
        if matches!(app.phase, Phase::Answering) {
            let q_len = app.current_question.chars().count();
            if app.reveal < q_len {
                app.reveal = (app.reveal + 3).min(q_len);
            }
        }

        terminal.draw(|f| ui(f, &app)).ok();

        if app.should_quit {
            break;
        }

        // Poll events with short timeout for animations
        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            if let Ok(ev) = event::read() {
                handle_event(&mut app, ev);
            }
        }

        app.tick += 1;
    }

    restore_terminal(&mut terminal);
}
