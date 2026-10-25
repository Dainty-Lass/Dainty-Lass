//! Split-view `ratatui` terminal dashboard (`--tui`) for `server-console*` binaries (`docs/app-structure-plan.md`).
//!
//! Displays live component statuses from `StatusRegistry`, a filterable ring-buffer log viewer fed
//! by `TuiRingBufferLayer`, and an interactive command bar supporting all REPL commands.

use std::io;
use std::time::Duration;

use controller::ServerController;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use service_kit::context::ComponentPhase;
use service_kit::telemetry::{LogFilterReloadHandle, TuiRingBufferLayer};
use tokio::sync::watch;

use crate::repl::execute_command;

const SERVICE_FILTERS: &[Option<&str>] = &[
    None,
    Some("host"),
    Some("gateway"),
    Some("allowlist"),
    Some("content"),
    Some("admin"),
    Some("mongod"),
    Some("backend"),
    Some("director"),
];

struct TuiGuard;

impl Drop for TuiGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Runs the interactive `ratatui` dashboard until the operator quits (`stop`, `quit`, or `Ctrl+C`).
pub async fn run_tui(
    controller: &mut ServerController,
    ring_layer: TuiRingBufferLayer,
    filter_handle: Option<LogFilterReloadHandle>,
    stop: watch::Receiver<bool>,
) -> Result<(), String> {
    enable_raw_mode().map_err(|e| format!("failed to enable terminal raw mode: {e}"))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| format!("failed to enter alternate screen: {e}"))?;
    let _guard = TuiGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| format!("failed to create ratatui terminal: {e}"))?;

    let mut input = String::new();
    let mut filter_idx: usize = 0;
    let mut scroll_offset: usize = 0;
    let mut status_note = String::from("Type `help` for commands | Tab: cycle service filter | Up/Down: scroll logs");

    loop {
        if *stop.borrow() {
            break;
        }
        let snap = controller.snapshot();
        let active_filter = SERVICE_FILTERS.get(filter_idx).copied().flatten();
        let entries = ring_layer.entries(active_filter);

        terminal
            .draw(|frame| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length((snap.components.len() as u16).saturating_add(4).max(7)),
                        Constraint::Min(8),
                        Constraint::Length(3),
                    ])
                    .split(frame.area());

                // Top pane: Component health table
                let header = Row::new(vec![
                    Cell::from("Component"),
                    Cell::from("Phase"),
                    Cell::from("Port"),
                    Cell::from("Uptime"),
                    Cell::from("Detail"),
                ])
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

                let rows: Vec<Row> = snap
                    .components
                    .iter()
                    .map(|comp| {
                        let phase_color = match comp.phase {
                            ComponentPhase::Ready => Color::Green,
                            ComponentPhase::Starting | ComponentPhase::Stopping => Color::Yellow,
                            ComponentPhase::Failed => Color::Red,
                            ComponentPhase::Stopped | ComponentPhase::Disabled => Color::DarkGray,
                        };
                        let port_str = comp.port.map_or_else(|| "-".into(), |p| p.to_string());
                        Row::new(vec![
                            Cell::from(comp.name.clone()),
                            Cell::from(comp.phase.as_str()).style(Style::default().fg(phase_color)),
                            Cell::from(port_str),
                            Cell::from(format!("{}s", comp.uptime_secs)),
                            Cell::from(comp.detail.clone()),
                        ])
                    })
                    .collect();

                let title = format!(
                    " Reforged Revival Server | {} | Players: {} | Hunts: {}/{} | fp: {} ",
                    snap.mode,
                    snap.online_players,
                    snap.active_hunts,
                    snap.max_hunts,
                    snap.cert_fingerprint.as_deref().unwrap_or("-")
                );
                let table = Table::new(
                    rows,
                    [
                        Constraint::Length(20),
                        Constraint::Length(12),
                        Constraint::Length(8),
                        Constraint::Length(10),
                        Constraint::Min(20),
                    ],
                )
                .header(header)
                .block(Block::default().borders(Borders::ALL).title(title));
                frame.render_widget(table, chunks[0]);

                // Middle pane: Live ring-buffer logs
                let visible_height = chunks[1].height.saturating_sub(2) as usize;
                let total = entries.len();
                let end = total.saturating_sub(scroll_offset);
                let start = end.saturating_sub(visible_height);
                let log_lines: Vec<Line> = entries[start..end]
                    .iter()
                    .map(|e| {
                        let lvl_color = match e.level.as_str() {
                            "error" => Color::Red,
                            "warn" => Color::Yellow,
                            "debug" | "trace" => Color::DarkGray,
                            _ => Color::Cyan,
                        };
                        Line::from(vec![
                            Span::styled(
                                format!("{:<14}", e.service),
                                Style::default().fg(lvl_color).add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" | "),
                            Span::raw(e.message.clone()),
                        ])
                    })
                    .collect();

                let filter_label = active_filter.unwrap_or("ALL");
                let log_block = Paragraph::new(log_lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Live Logs [filter: {filter_label}] ({total} lines) ")),
                );
                frame.render_widget(log_block, chunks[1]);

                // Bottom pane: Command bar
                let cmd_block = Paragraph::new(format!("> {input}")).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Command ({status_note}) ")),
                );
                frame.render_widget(cmd_block, chunks[2]);
            })
            .map_err(|e| format!("ratatui draw error: {e}"))?;

        if event::poll(Duration::from_millis(50)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                break;
            }
            match key.code {
                KeyCode::Tab => {
                    filter_idx = (filter_idx + 1) % SERVICE_FILTERS.len();
                    scroll_offset = 0;
                }
                KeyCode::Up => {
                    scroll_offset = scroll_offset.saturating_add(1);
                }
                KeyCode::Down => {
                    scroll_offset = scroll_offset.saturating_sub(1);
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Enter => {
                    let cmd = std::mem::take(&mut input);
                    let (replies, quit) = execute_command(&cmd, controller, filter_handle.as_ref()).await;
                    if let Some(last) = replies.last() {
                        status_note.clone_from(last);
                    }
                    for reply in replies {
                        tracing::info!(target: "host", "{reply}");
                    }
                    if quit {
                        break;
                    }
                }
                KeyCode::Char(c) => {
                    input.push(c);
                }
                _ => {}
            }
        }
    }
    Ok(())
}
