use anyhow::{anyhow, Result};
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
    Terminal,
};
use std::{
    io,
    sync::mpsc::{self, TryRecvError},
    thread,
    time::{Duration, Instant},
};

mod app;
mod engine;
mod session;
mod ui;
mod workbench;
#[cfg(feature = "web")]
mod web_server;

use app::{App, Mode};

fn main() -> Result<()> {
    match parse_args(std::env::args().skip(1))? {
        LaunchMode::Tui { db_path } => run_tui(&db_path),
        #[cfg(feature = "web")]
        LaunchMode::Web { db_path, bind_addr } => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(web_server::serve(db_path, bind_addr))
        }
        #[cfg(not(feature = "web"))]
        LaunchMode::Web { .. } => Err(anyhow!(
            "ndbstudio was built without the `web` feature; rebuild with `cargo run -p ndbstudio --features web -- --web <db-path>`"
        )),
    }
}

fn run_tui(db_path: &str) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _terminal_guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Open database with animated loading screen
    let mut app = if std::env::var("NDBSTUDIO_NO_LOADING").is_ok() {
        App::new(db_path)?
    } else {
        open_app_with_loading(&mut terminal, db_path)?
    };

    // Main event loop
    let res = run_app(&mut terminal, &mut app);

    if let Err(err) = res {
        eprintln!("Error: {:?}", err);
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum LaunchMode {
    Tui { db_path: String },
    Web { db_path: Option<String>, bind_addr: String },
}

fn parse_args<I>(args: I) -> Result<LaunchMode>
where
    I: IntoIterator<Item = String>,
{
    let mut db_path: Option<String> = None;
    let mut web_mode = false;
    let mut bind_addr = "127.0.0.1:3737".to_string();

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--web" => web_mode = true,
            "--bind" => {
                let Some(value) = iter.next() else {
                    return Err(anyhow!("--bind requires an address like 127.0.0.1:3737"));
                };
                bind_addr = value;
            }
            "--help" | "-h" => {
                print_usage_and_exit(0);
            }
            value if value.starts_with("--") => {
                return Err(anyhow!("unknown flag: {}", value));
            }
            value => {
                if db_path.is_some() {
                    return Err(anyhow!("only one database path is allowed"));
                }
                db_path = Some(value.to_string());
            }
        }
    }

    if web_mode {
        Ok(LaunchMode::Web { db_path, bind_addr })
    } else {
        let Some(db_path) = db_path else {
            print_usage_and_exit(1);
        };
        Ok(LaunchMode::Tui { db_path })
    }
}

fn print_usage_and_exit(code: i32) -> ! {
    eprintln!("NDStudio - Interactive explorer for NopalDB");
    eprintln!("Usage: ndstudio <database_path>");
    eprintln!("       ndstudio --web [database_path] [--bind 127.0.0.1:3737]");
    eprintln!("\nExamples:");
    eprintln!("  ndstudio ./my_graph.db");
    eprintln!("  ndstudio --web");
    eprintln!("  ndstudio --web ./my_graph.db");
    eprintln!("\nFor more info: https://github.com/sharop/nopaldb");
    std::process::exit(code);
}

fn open_app_with_loading<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    db_path: &str,
) -> Result<App>
where
    B::Error: Send + Sync + 'static,
{
    let (tx, rx) = mpsc::channel();
    let db_path_owned = db_path.to_string();

    thread::spawn(move || {
        let _ = tx.send(App::open_graph(&db_path_owned));
    });

    let started_at = Instant::now();
    let mut tick: usize = 0;

    loop {
        match rx.try_recv() {
            Ok(Ok(graph)) => return App::from_graph(db_path, graph),
            Ok(Err(err)) => return Err(err),
            Err(TryRecvError::Disconnected) => {
                return Err(anyhow!("database loader thread disconnected unexpectedly"));
            }
            Err(TryRecvError::Empty) => {}
        }

        terminal.draw(|f| draw_loading_screen(f, db_path, started_at.elapsed(), tick))?;
        tick = tick.wrapping_add(1);
        thread::sleep(Duration::from_millis(90));
    }
}

fn draw_loading_screen(f: &mut Frame, db_path: &str, elapsed: Duration, tick: usize) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(18),
            Constraint::Length(22),
            Constraint::Percentage(60),
        ])
        .split(area);

    let block = Block::default()
        .title(" NDStudio ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui::ACCENT));
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);

    let spinner = ["|", "/", "-", "\\"];
    let spinner_char = spinner[tick % spinner.len()];
    let track_len = 22usize;
    let tuna_pos = tick % track_len;
    let mut track = vec!['.'; track_len];
    track[tuna_pos] = 'o';
    let track_line: String = track.into_iter().collect();

    let elapsed_s = elapsed.as_secs_f32();
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} Cargando las tunas al nopal...", spinner_char),
                Style::default().fg(ui::ACCENT).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            format!("DB: {}", db_path),
            Style::default().fg(ui::FG),
        )),
        Line::from(Span::styled(
            format!("Tiempo: {:.1}s", elapsed_s),
            Style::default().fg(ui::FG),
        )),
        Line::from(" "),
        Line::from(Span::styled(
            format!("  [{}]=>  cargando...", track_line),
            Style::default().fg(ui::SUCCESS),
        )),
        Line::from(" "),
    ];
    lines.extend(animated_logo_lines(tick));
    lines.push(Line::from(" "));
    lines.push(animated_brand_line(tick));
    lines.push(Line::from(Span::styled(
        "        Graph Engine Loading",
        Style::default().fg(Color::Rgb(95, 205, 170)),
    )));

    let loading = Paragraph::new(lines);
    f.render_widget(loading, inner);
}

fn animated_logo_lines(tick: usize) -> Vec<Line<'static>> {
    const TEMPLATE: [&str; 10] = [
        "         t      t         ",
        "      ..gggggggg..t       ",
        "   ...gggggggggggg....    ",
        " ....gggggssssggggg...    ",
        "...gggggggssssgggggg..t   ",
        " ..gggggggggggggggg..     ",
        "   ..gggggggggggg..       ",
        "      ...gggg....         ",
        "          ||              ",
        "         _||_             ",
    ];

    let cycle = 96usize;
    let phase = tick % cycle;
    let sweep = if phase < cycle / 2 {
        phase * 100 / (cycle / 2)
    } else {
        (cycle - phase) * 100 / (cycle / 2)
    };

    TEMPLATE
        .iter()
        .enumerate()
        .map(|(y, row)| {
            let width = row.len().max(1);
            let spans = row
                .chars()
                .enumerate()
                .map(|(x, ch)| {
                    let draw_char = render_pixel(ch, x, y, width, sweep, tick);
                    Span::styled(
                        draw_char.to_string(),
                        pixel_style(ch, x, y, tick),
                    )
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect()
}

fn render_pixel(ch: char, x: usize, y: usize, width: usize, sweep: usize, tick: usize) -> char {
    if ch == ' ' {
        return ' ';
    }

    let x_pct = (x * 100) / width;
    if x_pct <= sweep {
        if ch == 't' && (tick + x + y).is_multiple_of(7) {
            '*'
        } else {
            ch
        }
    } else if x_pct <= sweep + 12 && (tick + x * 3 + y * 5).is_multiple_of(4) {
        '.'
    } else {
        ' '
    }
}

fn pixel_style(ch: char, x: usize, y: usize, tick: usize) -> Style {
    match ch {
        'g' => {
            if (x + y + tick).is_multiple_of(5) {
                Style::default().fg(ui::ACCENT)
            } else {
                Style::default().fg(ui::SUCCESS)
            }
        }
        't' | '*' => Style::default().fg(Color::Rgb(240, 86, 52)).add_modifier(Modifier::BOLD),
        's' => Style::default().fg(Color::Rgb(230, 190, 85)),
        '|' | '_' => Style::default().fg(Color::Rgb(120, 180, 110)),
        '.' => Style::default().fg(Color::Rgb(70, 180, 170)),
        _ => Style::default().fg(ui::FG),
    }
}

fn animated_brand_line(tick: usize) -> Line<'static> {
    let text = "NopalDB";
    let reveal = tick % (text.len() + 6);
    let spans = text
        .chars()
        .enumerate()
        .map(|(idx, ch)| {
            let style = if idx <= reveal {
                if (tick + idx).is_multiple_of(3) {
                    Style::default()
                        .fg(Color::Rgb(86, 231, 170))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Rgb(42, 197, 176))
                        .add_modifier(Modifier::BOLD)
                }
            } else {
                Style::default().fg(Color::Rgb(45, 70, 72))
            };
            Span::styled(ch.to_string(), style)
        })
        .collect::<Vec<_>>();

    let mut centered = vec![Span::styled(
        "         ",
        Style::default().fg(Color::Rgb(45, 70, 72)),
    )];
    centered.extend(spans);
    Line::from(centered)
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    loop {
        if app.quit_requested() {
            break;
        }

        // Draw UI
        terminal.draw(|f| ui::draw(f, app))?;

        // Handle events
        if event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            // Global quit handlers
            if should_quit(key, app.mode()) {
                break;
            }

            // Pass key to app
            app.handle_key(key)?;
        }

        // Allow app to do background work
        app.tick()?;
    }

    Ok(())
}

fn should_quit(key: KeyEvent, mode: Mode) -> bool {
    // Quit on 'q' in Normal mode
    if matches!(mode, Mode::Normal) && key.code == KeyCode::Char('q') {
        return true;
    }

    // Quit on Ctrl+C anywhere
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    false
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, Show);
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_args, LaunchMode};

    #[test]
    fn parse_tui_mode_from_db_path() {
        let mode = parse_args(vec!["demo.db".to_string()]).expect("args should parse");
        assert_eq!(
            mode,
            LaunchMode::Tui {
                db_path: "demo.db".to_string(),
            }
        );
    }

    #[test]
    fn parse_web_mode_with_default_bind() {
        let mode = parse_args(vec!["--web".to_string(), "demo.db".to_string()])
            .expect("args should parse");
        assert_eq!(
            mode,
            LaunchMode::Web {
                db_path: Some("demo.db".to_string()),
                bind_addr: "127.0.0.1:3737".to_string(),
            }
        );
    }

    #[test]
    fn parse_web_mode_without_db_path() {
        let mode = parse_args(vec!["--web".to_string()]).expect("args should parse");
        assert_eq!(
            mode,
            LaunchMode::Web {
                db_path: None,
                bind_addr: "127.0.0.1:3737".to_string(),
            }
        );
    }

    #[test]
    fn parse_web_mode_with_custom_bind() {
        let mode = parse_args(vec![
            "--web".to_string(),
            "--bind".to_string(),
            "127.0.0.1:4040".to_string(),
            "demo.db".to_string(),
        ])
        .expect("args should parse");
        assert_eq!(
            mode,
            LaunchMode::Web {
                db_path: Some("demo.db".to_string()),
                bind_addr: "127.0.0.1:4040".to_string(),
            }
        );
    }
}
