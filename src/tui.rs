use crate::state::{Action, HintOp, Mode, Outcome, State};
use crate::tokens;
use crate::tokens::QuoteStyle;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
};
use std::io::{Stderr, stderr};
use std::time::{Duration, Instant};

const DEFAULT_STATUS: &str =
    "hint edit · a/i add · d delete · u undo · ^R redo · Enter accept · Esc cancel";
const STATUS_TTL: Duration = Duration::from_millis(1800);

// Palette — deliberately restrained.
const BORDER: Color = Color::DarkGray;
const TITLE: Color = Color::White;
const ACCENT: Color = Color::Yellow;
const HIGHLIGHT: Color = Color::Cyan;
const SUBTLE: Color = Color::Gray;

type Term = Terminal<CrosstermBackend<Stderr>>;

struct TermGuard {
    terminal: Term,
}

impl TermGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(stderr(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stderr());
        let mut terminal = Terminal::new(backend)?;
        terminal.hide_cursor()?;
        terminal.clear()?;
        Ok(Self { terminal })
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}

fn read_key() -> Result<KeyEvent> {
    loop {
        if let Event::Key(k) = event::read()? {
            if k.kind == event::KeyEventKind::Release {
                continue;
            }
            return Ok(k);
        }
    }
}

fn read_key_until(deadline: Instant) -> Result<Option<KeyEvent>> {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        if event::poll(deadline - now)? {
            if let Event::Key(k) = event::read()? {
                if k.kind == event::KeyEventKind::Release {
                    continue;
                }
                return Ok(Some(k));
            }
        } else {
            return Ok(None);
        }
    }
}

// ────────────────────────────── picker ──────────────────────────────

pub fn pick_entry(entries: &[String]) -> Result<Option<String>> {
    let mut guard = TermGuard::enter()?;
    let mut sel: usize = 0;
    let mut list_state = ListState::default();
    list_state.select(Some(sel));

    loop {
        guard
            .terminal
            .draw(|f| draw_picker(f, entries, sel, &mut list_state))?;

        let k = read_key()?;
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(None),
            KeyCode::Up | KeyCode::Char('k') => sel = sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if sel + 1 < entries.len() {
                    sel += 1;
                }
            }
            KeyCode::PageUp => sel = sel.saturating_sub(10),
            KeyCode::PageDown => sel = (sel + 10).min(entries.len().saturating_sub(1)),
            KeyCode::Home => sel = 0,
            KeyCode::End => sel = entries.len().saturating_sub(1),
            KeyCode::Enter => return Ok(Some(entries[sel].clone())),
            _ => {}
        }
        list_state.select(Some(sel));
    }
}

fn draw_picker(f: &mut Frame, entries: &[String], sel: usize, list_state: &mut ListState) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let is_sel = i == sel;
            let marker = if is_sel { "▸ " } else { "  " };
            let text_style = if is_sel {
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            let idx_style = Style::new().fg(SUBTLE).add_modifier(Modifier::DIM);
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::new().fg(ACCENT)),
                Span::styled(format!("{:>3}  ", i + 1), idx_style),
                Span::styled(cmd.clone(), text_style),
            ]))
        })
        .collect();

    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            "tweaker",
            Style::new().fg(TITLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · history ", Style::new().fg(SUBTLE)),
    ]);

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(BORDER))
            .title(title),
    );
    f.render_stateful_widget(list, chunks[0], list_state);

    let help = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled("↑/↓", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(" navigate  ", Style::new().fg(SUBTLE)),
        Span::styled(
            "Enter",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" select  ", Style::new().fg(SUBTLE)),
        Span::styled("Esc", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(" cancel", Style::new().fg(SUBTLE)),
    ]));
    f.render_widget(help, chunks[1]);
}

// ────────────────────────────── tweak ──────────────────────────────

pub fn tweak(cmd: &str) -> Result<Option<String>> {
    let initial = tokens::split(cmd)?;
    let mut state = State::new(initial);
    let mut guard = TermGuard::enter()?;
    let mut status_expires: Option<Instant> = None;

    loop {
        guard.terminal.draw(|f| draw_tweak(f, &state))?;

        let k = match status_expires {
            Some(deadline) => match read_key_until(deadline)? {
                Some(k) => k,
                None => {
                    state.status = None;
                    status_expires = None;
                    continue;
                }
            },
            None => read_key()?,
        };

        let Some(action) = key_to_action(&k, &state.mode) else {
            continue;
        };

        match state.apply(action) {
            Outcome::Quit => return Ok(None),
            Outcome::Accept => return Ok(Some(tokens::render(&state.tokens))),
            Outcome::Continue => {}
        }

        status_expires = match state.mode {
            Mode::Normal if state.status.is_some() => Some(Instant::now() + STATUS_TTL),
            _ => None,
        };
    }
}

fn key_to_action(k: &KeyEvent, mode: &Mode) -> Option<Action> {
    use KeyCode::*;
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match (&k.code, mode) {
        (Esc, _) => Some(Action::Cancel),
        (Char('c'), _) if ctrl => Some(Action::Cancel),

        // Editing mode takes character keys for the buffer.
        (Enter, Mode::Editing { .. }) => Some(Action::Commit),
        (Char('u'), Mode::Editing { .. }) if ctrl => Some(Action::ClearLine),
        (Char('s'), Mode::Editing { .. }) if ctrl => Some(Action::ToggleQuote),
        (Backspace, Mode::Editing { .. }) => Some(Action::Backspace),
        (Delete, Mode::Editing { .. }) => Some(Action::Delete),
        (Left, Mode::Editing { .. }) => Some(Action::Left),
        (Right, Mode::Editing { .. }) => Some(Action::Right),
        (Home, Mode::Editing { .. }) => Some(Action::Home),
        (End, Mode::Editing { .. }) => Some(Action::End),
        (Char(c), Mode::Editing { .. }) if !ctrl => Some(Action::Char(*c)),

        // Normal / AwaitHint.
        (Enter, _) => Some(Action::Commit),
        (Char('r'), Mode::Normal) if ctrl => Some(Action::Redo),
        (Char('u'), Mode::Normal) => Some(Action::Undo),
        (Char('d'), Mode::Normal) => Some(Action::Prefix(HintOp::Delete)),
        (Char('a'), Mode::Normal) => Some(Action::Prefix(HintOp::InsertAfter)),
        (Char('i'), Mode::Normal) => Some(Action::Prefix(HintOp::InsertBefore)),
        (Char(ch), _) => Some(Action::Hint(*ch)),

        _ => None,
    }
}

fn draw_tweak(f: &mut Frame, state: &State) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(5), // tweak box
        Constraint::Min(0),    // filler
        Constraint::Length(1), // status
    ])
    .split(area);

    let (mode_label, mode_style) = match &state.mode {
        Mode::Normal => ("normal", Style::new().fg(SUBTLE)),
        Mode::AwaitHint(_) => ("pending", Style::new().fg(HIGHLIGHT)),
        Mode::Editing { .. } => ("editing", Style::new().fg(ACCENT)),
    };

    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            "tweaker",
            Style::new().fg(TITLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::new().fg(SUBTLE)),
        Span::styled(mode_label, mode_style.add_modifier(Modifier::BOLD)),
        Span::raw(" "),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(title);

    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);

    // Inside the box: [blank, command, hints] with 2-col left padding.
    let pad = 2u16;
    let content_x = inner.x + pad;
    let content_w = inner.width.saturating_sub(pad * 2);
    let cmd_rect = Rect {
        x: content_x,
        y: inner.y + 1,
        width: content_w,
        height: 1,
    };
    let hint_rect = Rect {
        x: content_x,
        y: inner.y + 2,
        width: content_w,
        height: 1,
    };

    let (cmd_line, hint_line, cursor_col) = build_cmd_view(state);
    f.render_widget(Paragraph::new(cmd_line), cmd_rect);
    f.render_widget(Paragraph::new(hint_line), hint_rect);

    // Status bar below the box.
    let status_line = status_line(state);
    f.render_widget(Paragraph::new(status_line), chunks[2]);

    if let Some(col) = cursor_col {
        f.set_cursor_position(Position {
            x: cmd_rect.x + col as u16,
            y: cmd_rect.y,
        });
    }
}

fn build_cmd_view(state: &State) -> (Line<'static>, Line<'static>, Option<usize>) {
    match &state.mode {
        Mode::Editing {
            idx,
            buf,
            cursor,
            quote_style,
            ..
        } => build_editing_view(state, *idx, buf, *cursor, *quote_style),
        _ => build_hint_view(state),
    }
}

fn build_hint_view(state: &State) -> (Line<'static>, Line<'static>, Option<usize>) {
    let (rendered, spans) = tokens::render_with_spans(&state.tokens);
    let cmd = Line::from(Span::styled(
        rendered,
        Style::new().add_modifier(Modifier::BOLD),
    ));

    let mut hint_spans: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for (i, (start, _len)) in spans.iter().enumerate() {
        let Some(lbl) = tokens::label(i) else { break };
        if *start > col {
            hint_spans.push(Span::raw(" ".repeat(start - col)));
        }
        hint_spans.push(Span::styled(
            lbl.to_string(),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
        col = start + 1;
    }

    (cmd, Line::from(hint_spans), None)
}

fn build_editing_view(
    state: &State,
    idx: usize,
    buf: &[char],
    cursor: usize,
    quote_style: QuoteStyle,
) -> (Line<'static>, Line<'static>, Option<usize>) {
    let mut before = String::new();
    let mut after = String::new();
    let raw: String = buf.iter().collect();
    let mut token_col = 0usize;
    let mut past_edited = false;

    for (i, t) in state.tokens.iter().enumerate() {
        let sep = if i > 0 { " " } else { "" };
        if i == idx {
            before.push_str(sep);
            token_col = before.chars().count();
            past_edited = true;
            continue;
        }
        if past_edited {
            after.push_str(sep);
            after.push_str(&t.original);
        } else {
            before.push_str(sep);
            before.push_str(&t.original);
        }
    }

    // Show the token with the current quote wrapping (no escaping — that happens on commit).
    let (open, close) = match quote_style {
        QuoteStyle::None => ("", ""),
        QuoteStyle::Single => ("'", "'"),
        QuoteStyle::Double => ("\"", "\""),
    };
    let token_display = format!("{open}{raw}{close}");

    let cmd = Line::from(vec![
        Span::styled(before, Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(
            token_display,
            Style::new()
                .fg(ACCENT)
                .add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
        ),
        Span::styled(after, Style::new().add_modifier(Modifier::BOLD)),
    ]);

    let hints = Line::from(Span::styled(
        "↵ commit  ⎋ cancel  ^U clear  ^S quote",
        Style::new().fg(SUBTLE).add_modifier(Modifier::DIM),
    ));

    // Cursor offset: token column + optional opening quote + position in buffer.
    (
        cmd,
        hints,
        Some(token_col + quote_style.prefix_len() + cursor),
    )
}

fn status_line(state: &State) -> Line<'static> {
    match (&state.status, &state.mode) {
        (_, Mode::AwaitHint(op)) => {
            let op_char = match op {
                HintOp::Delete => "d",
                HintOp::InsertBefore => "i",
                HintOp::InsertAfter => "a",
            };
            let word = match op {
                HintOp::Delete => "delete",
                HintOp::InsertBefore => "insert before",
                HintOp::InsertAfter => "insert after",
            };
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    "› ",
                    Style::new().fg(HIGHLIGHT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    op_char.to_string(),
                    Style::new().fg(HIGHLIGHT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" — press a hint to {word}  "),
                    Style::new().fg(SUBTLE),
                ),
                Span::styled("Esc", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(" cancel", Style::new().fg(SUBTLE)),
            ])
        }
        (_, Mode::Editing { quote_style, .. }) => Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "✎ editing  ",
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                quote_style.label(),
                Style::new().fg(HIGHLIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Enter",
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" commit  ", Style::new().fg(SUBTLE)),
            Span::styled("Esc", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" cancel  ", Style::new().fg(SUBTLE)),
            Span::styled("^S", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" toggle quote  ", Style::new().fg(SUBTLE)),
            Span::styled("^U", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" clear", Style::new().fg(SUBTLE)),
        ]),
        (Some(msg), Mode::Normal) => Line::from(vec![
            Span::raw(" "),
            Span::styled("✓ ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(msg.clone(), Style::new().fg(ACCENT)),
        ]),
        (None, Mode::Normal) => Line::from(Span::styled(
            format!(" {DEFAULT_STATUS}"),
            Style::new().fg(SUBTLE).add_modifier(Modifier::DIM),
        )),
    }
}
