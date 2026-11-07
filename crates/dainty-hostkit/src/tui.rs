// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The dashboard of `serve --tui` (`docs/app-structure-plan.md`): the services' state from the
//! controller's registry, a log viewer fed by `TuiRingBufferLayer`, and a command prompt that runs the
//! console's commands ([`crate::repl::parse_command`]).
//!
//! Keys: Up/Down pick a service, `r` restarts it, `s` starts or stops it, `f` (or Tab) cycles the log
//! filter, PgUp/PgDn scroll the log, `:` opens the prompt (Enter runs, Esc closes), `q`, Esc or Ctrl+C
//! stops the server. [`on_key`] holds that map, apart from the terminal, so it is tested.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use dainty_controller::ServerController;
use dainty_service_kit::context::ComponentPhase;
use dainty_service_kit::telemetry::{LogFilterReloadHandle, TuiRingBufferLayer};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use tokio::sync::watch;

use crate::repl::{ConsoleCommand, MAX_LINE, parse_command, run_command};

/// Lines PgUp and PgDn move the log by.
const PAGE: usize = 10;

const KEYS: &str = "↑/↓ pick · r restart · s start/stop · f filter · PgUp/PgDn scroll · : command · q quit";

struct TuiGuard;

impl Drop for TuiGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// What the dashboard shows apart from the server's own state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TuiState {
    /// The service the keys act on, an index into the table
    pub selected: usize,
    /// 0 is every service's log; n is the n-th service's
    pub filter: usize,
    /// Lines back from the newest
    pub scroll: usize,
    /// The command prompt, while it is open
    pub prompt: Option<String>,
    /// The last line a command or a key said
    pub note: String,
}

/// What a key asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Nothing to do.
    None,
    /// Runs a console command.
    Run(ConsoleCommand),
    /// Stops the server.
    Quit,
}

/// One key, given the services as the table lists them (name, running or not).
pub fn on_key(state: &mut TuiState, key: KeyEvent, services: &[(String, bool)]) -> KeyAction {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return KeyAction::Quit;
    }
    if let Some(input) = state.prompt.as_mut() {
        match key.code {
            KeyCode::Esc => state.prompt = None,
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => {
                let line = state.prompt.take().unwrap_or_default();
                return match parse_command(&line) {
                    Ok(ConsoleCommand::Quit) => KeyAction::Quit,
                    Ok(ConsoleCommand::Empty) => KeyAction::None,
                    Ok(command) => KeyAction::Run(command),
                    Err(why) => {
                        state.note = why;
                        KeyAction::None
                    }
                };
            }
            // Only text: a control character typed at the prompt is never drawn
            KeyCode::Char(c) if !c.is_control() && input.len() < MAX_LINE => input.push(c),
            _ => {}
        }
        return KeyAction::None;
    }
    let chosen = services.get(state.selected);
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return KeyAction::Quit,
        KeyCode::Up => state.selected = state.selected.saturating_sub(1),
        KeyCode::Down if state.selected + 1 < services.len() => state.selected += 1,
        KeyCode::Char('r') => {
            if let Some((name, _)) = chosen {
                return KeyAction::Run(ConsoleCommand::Restart(name.clone()));
            }
        }
        KeyCode::Char('s') => {
            if let Some((name, running)) = chosen {
                return KeyAction::Run(if *running {
                    ConsoleCommand::Stop(name.clone())
                } else {
                    ConsoleCommand::Start(name.clone())
                });
            }
        }
        KeyCode::Char('f') | KeyCode::Tab => {
            state.filter = (state.filter + 1) % (services.len() + 1);
            state.scroll = 0;
        }
        KeyCode::PageUp => state.scroll = state.scroll.saturating_add(PAGE),
        KeyCode::PageDown => state.scroll = state.scroll.saturating_sub(PAGE),
        KeyCode::Char(':') => state.prompt = Some(String::new()),
        _ => {}
    }
    KeyAction::None
}

fn phase_color(phase: ComponentPhase) -> Color {
    match phase {
        ComponentPhase::Ready => Color::Green,
        ComponentPhase::Starting | ComponentPhase::Stopping => Color::Yellow,
        ComponentPhase::Failed => Color::Red,
        ComponentPhase::Stopped | ComponentPhase::Disabled => Color::DarkGray,
    }
}

/// Runs the dashboard until the operator quits or a stop signal comes.
pub async fn run_tui(
    controller: &mut ServerController,
    ring_layer: TuiRingBufferLayer,
    filter_handle: Option<LogFilterReloadHandle>,
    workspace: Option<&crate::paths::Paths>,
    stop: watch::Receiver<bool>,
) -> Result<(), String> {
    enable_raw_mode().map_err(|e| format!("failed to enable terminal raw mode: {e}"))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| format!("failed to enter alternate screen: {e}"))?;
    let _guard = TuiGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| format!("failed to create ratatui terminal: {e}"))?;
    let mut state = TuiState {
        note: KEYS.into(),
        ..TuiState::default()
    };

    loop {
        if *stop.borrow() {
            break;
        }
        let snap = controller.snapshot();
        let services: Vec<(String, bool)> = snap
            .components
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    matches!(c.phase, ComponentPhase::Ready | ComponentPhase::Starting),
                )
            })
            .collect();
        state.selected = state.selected.min(services.len().saturating_sub(1));
        let filter = state
            .filter
            .checked_sub(1)
            .and_then(|i| services.get(i))
            .map(|(name, _)| name.as_str());
        let entries = ring_layer.entries(filter);

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

                let header = Row::new(vec![
                    Cell::from("Service"),
                    Cell::from("Phase"),
                    Cell::from("Port"),
                    Cell::from("Uptime"),
                    Cell::from("Detail"),
                ])
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
                let rows: Vec<Row> = snap
                    .components
                    .iter()
                    .enumerate()
                    .map(|(i, comp)| {
                        let row = Row::new(vec![
                            Cell::from(comp.name.clone()),
                            Cell::from(comp.phase.as_str()).style(Style::default().fg(phase_color(comp.phase))),
                            Cell::from(comp.port.map_or_else(|| "-".into(), |p| p.to_string())),
                            Cell::from(format!("{}s", comp.uptime_secs)),
                            Cell::from(comp.detail.clone()),
                        ]);
                        if i == state.selected {
                            row.style(Style::default().add_modifier(Modifier::REVERSED))
                        } else {
                            row
                        }
                    })
                    .collect();
                let title = format!(
                    " Dainty Lass Server | {} | Players: {} | Hunts: {}/{} | fp: {} ",
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

                let visible = chunks[1].height.saturating_sub(2) as usize;
                let total = entries.len();
                let end = total.saturating_sub(state.scroll);
                let start = end.saturating_sub(visible);
                let log_lines: Vec<Line> = entries[start..end]
                    .iter()
                    .map(|e| {
                        let color = match e.level.as_str() {
                            "error" => Color::Red,
                            "warn" => Color::Yellow,
                            "debug" | "trace" => Color::DarkGray,
                            _ => Color::Cyan,
                        };
                        Line::from(vec![
                            Span::styled(
                                format!("{:<14}", e.service),
                                Style::default().fg(color).add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" | "),
                            Span::raw(crate::repl::plain(&e.message)),
                        ])
                    })
                    .collect();
                let log_block = Paragraph::new(log_lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Log [{}] ({total} lines) ", filter.unwrap_or("all"))),
                );
                frame.render_widget(log_block, chunks[1]);

                let bar = match &state.prompt {
                    Some(input) => Paragraph::new(format!(":{input}")).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Command (Enter runs, Esc closes) "),
                    ),
                    None => Paragraph::new(state.note.clone()).block(Block::default().borders(Borders::ALL)),
                };
                frame.render_widget(bar, chunks[2]);
            })
            .map_err(|e| format!("ratatui draw error: {e}"))?;

        if event::poll(Duration::from_millis(50)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            match on_key(&mut state, key, &services) {
                KeyAction::None => {}
                KeyAction::Quit => break,
                KeyAction::Run(command) => {
                    let (replies, quit) = run_command(command, controller, filter_handle.as_ref(), workspace).await;
                    if let Some(last) = replies.last() {
                        state.note.clone_from(last);
                    }
                    for reply in replies {
                        tracing::info!(target: "host", "{reply}");
                    }
                    if quit {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn services() -> Vec<(String, bool)> {
        vec![
            ("content".into(), true),
            ("admin".into(), false),
            ("gateway".into(), true),
        ]
    }

    #[test]
    fn the_keys_pick_restart_start_stop_filter_and_scroll() {
        let s = services();
        let mut state = TuiState::default();
        assert_eq!(on_key(&mut state, key(KeyCode::Up), &s), KeyAction::None);
        assert_eq!(state.selected, 0, "not above the first");
        assert_eq!(
            on_key(&mut state, key(KeyCode::Char('r')), &s),
            KeyAction::Run(ConsoleCommand::Restart("content".into()))
        );
        on_key(&mut state, key(KeyCode::Down), &s);
        assert_eq!(
            on_key(&mut state, key(KeyCode::Char('s')), &s),
            KeyAction::Run(ConsoleCommand::Start("admin".into())),
            "a stopped one starts"
        );
        on_key(&mut state, key(KeyCode::Down), &s);
        on_key(&mut state, key(KeyCode::Down), &s);
        assert_eq!(state.selected, 2, "not below the last");
        assert_eq!(
            on_key(&mut state, key(KeyCode::Char('s')), &s),
            KeyAction::Run(ConsoleCommand::Stop("gateway".into())),
            "a running one stops"
        );
        for expected in [1, 2, 3, 0] {
            on_key(&mut state, key(KeyCode::Char('f')), &s);
            assert_eq!(state.filter, expected, "all, then each service, then all again");
        }
        on_key(&mut state, key(KeyCode::PageUp), &s);
        on_key(&mut state, key(KeyCode::PageUp), &s);
        on_key(&mut state, key(KeyCode::PageDown), &s);
        assert_eq!(state.scroll, PAGE);
        assert_eq!(on_key(&mut state, key(KeyCode::Char('q')), &s), KeyAction::Quit);
        assert_eq!(
            on_key(
                &mut TuiState::default(),
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &s
            ),
            KeyAction::Quit
        );
        assert_eq!(
            on_key(&mut TuiState::default(), key(KeyCode::Char('r')), &[]),
            KeyAction::None,
            "no service, nothing to restart"
        );
    }

    #[test]
    fn the_prompt_runs_a_command_and_takes_only_text() {
        let s = services();
        let mut state = TuiState::default();
        on_key(&mut state, key(KeyCode::Char(':')), &s);
        for c in "log admin debux".chars() {
            on_key(&mut state, key(KeyCode::Char(c)), &s);
        }
        on_key(&mut state, key(KeyCode::Backspace), &s);
        on_key(&mut state, key(KeyCode::Char('g')), &s);
        on_key(&mut state, key(KeyCode::Char('\u{1b}')), &s);
        assert_eq!(state.prompt.as_deref(), Some("log admin debug"), "no control character");
        assert_eq!(
            on_key(&mut state, key(KeyCode::Enter), &s),
            KeyAction::Run(ConsoleCommand::Log {
                service: "admin".into(),
                level: "debug"
            })
        );
        assert_eq!(state.prompt, None, "Enter closes it");
        assert_eq!(
            on_key(&mut state, key(KeyCode::Char('q')), &s),
            KeyAction::Quit,
            "keys again"
        );

        let mut state = TuiState::default();
        on_key(&mut state, key(KeyCode::Char(':')), &s);
        on_key(&mut state, key(KeyCode::Char('q')), &s);
        assert_eq!(state.prompt.as_deref(), Some("q"), "q is text while the prompt is open");
        on_key(&mut state, key(KeyCode::Enter), &s);
        assert!(state.note.contains("unknown command"), "{}", state.note);
        on_key(&mut state, key(KeyCode::Char(':')), &s);
        assert_eq!(
            on_key(&mut state, key(KeyCode::Esc), &s),
            KeyAction::None,
            "Esc closes the prompt"
        );
        assert_eq!(state.prompt, None);
    }
}
