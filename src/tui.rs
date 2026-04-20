use crate::state::{Action, HintOp, Mode, Outcome, State};
use crate::tokens;
use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{Write, stderr};
use std::time::{Duration, Instant};

const DEFAULT_STATUS: &str =
    "hint edit · a add after · i add before · d delete · Enter accept · Esc cancel";
const STATUS_TTL: Duration = Duration::from_millis(1800);

struct RawGuard;
impl RawGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(stderr(), EnterAlternateScreen, cursor::Hide)?;
        Ok(Self)
    }
}
impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = execute!(stderr(), cursor::Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
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

/// Like read_key, but returns None if `deadline` passes with no key.
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

pub fn pick_entry(entries: &[String]) -> Result<Option<String>> {
    let _g = RawGuard::enter()?;
    let mut sel: usize = 0;
    let mut out = stderr();
    loop {
        let (_, rows) = terminal::size()?;
        let visible = (rows as usize).saturating_sub(2).max(1);
        let start = sel.saturating_sub(visible - 1).min(entries.len().saturating_sub(visible));
        queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        queue!(
            out,
            SetAttribute(Attribute::Bold),
            Print("pick a command  (↑/↓ move, Enter select, q/Esc quit)\r\n"),
            SetAttribute(Attribute::Reset)
        )?;
        for (i, cmd) in entries.iter().enumerate().skip(start).take(visible) {
            if i == sel {
                queue!(
                    out,
                    SetForegroundColor(Color::Black),
                    crossterm::style::SetBackgroundColor(Color::White),
                    Print(format!("{:>3}  {}\r\n", i, truncate(cmd, 200))),
                    ResetColor
                )?;
            } else {
                queue!(out, Print(format!("{:>3}  {}\r\n", i, truncate(cmd, 200))))?;
            }
        }
        out.flush()?;

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
            KeyCode::Enter => return Ok(Some(entries[sel].clone())),
            _ => {}
        }
    }
}

const CMD_ROW: u16 = 2;
const CMD_COL: u16 = 2;

pub fn tweak(cmd: &str) -> Result<Option<String>> {
    let initial = tokens::split(cmd)?;
    let mut state = State::new(initial);
    let _g = RawGuard::enter()?;
    let mut out = stderr();
    let mut status_expires: Option<Instant> = None;

    loop {
        draw(&mut out, &state)?;

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
        (Esc, _) | (Char('c'), _) if matches!(k.code, Esc) || ctrl => Some(Action::Cancel),

        // Editing mode — character keys go into the buffer.
        (Enter, Mode::Editing { .. }) => Some(Action::Commit),
        (Char('u'), Mode::Editing { .. }) if ctrl => Some(Action::ClearLine),
        (Backspace, Mode::Editing { .. }) => Some(Action::Backspace),
        (Delete, Mode::Editing { .. }) => Some(Action::Delete),
        (Left, Mode::Editing { .. }) => Some(Action::Left),
        (Right, Mode::Editing { .. }) => Some(Action::Right),
        (Home, Mode::Editing { .. }) => Some(Action::Home),
        (End, Mode::Editing { .. }) => Some(Action::End),
        (Char(c), Mode::Editing { .. }) if !ctrl => Some(Action::Char(*c)),

        // Normal / AwaitHint.
        (Enter, _) => Some(Action::Commit),
        (Char('d'), Mode::Normal) => Some(Action::Prefix(HintOp::Delete)),
        (Char('a'), Mode::Normal) => Some(Action::Prefix(HintOp::InsertAfter)),
        (Char('i'), Mode::Normal) => Some(Action::Prefix(HintOp::InsertBefore)),
        (Char(ch), _) => Some(Action::Hint(*ch)),

        _ => None,
    }
}

fn draw<W: Write>(out: &mut W, state: &State) -> Result<()> {
    match &state.mode {
        Mode::Normal | Mode::AwaitHint(_) => draw_hints(out, state),
        Mode::Editing {
            idx, buf, cursor, ..
        } => draw_editing(out, state, *idx, buf, *cursor),
    }
}

fn draw_hints<W: Write>(out: &mut W, state: &State) -> Result<()> {
    let (rendered, spans) = tokens::render_with_spans(&state.tokens);
    queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    queue!(
        out,
        SetAttribute(Attribute::Bold),
        Print("tweak command\r\n"),
        SetAttribute(Attribute::Reset)
    )?;
    queue!(out, cursor::MoveTo(CMD_COL, CMD_ROW), Print(&rendered))?;

    for (i, (start, _len)) in spans.iter().enumerate() {
        let Some(lbl) = tokens::label(i) else { break };
        let col = CMD_COL + *start as u16;
        queue!(
            out,
            cursor::MoveTo(col, CMD_ROW + 1),
            SetForegroundColor(Color::Yellow),
            SetAttribute(Attribute::Bold),
            Print(lbl),
            ResetColor,
            SetAttribute(Attribute::Reset),
        )?;
    }

    write_status(out, &status_text(state))?;
    out.flush()?;
    Ok(())
}

fn draw_editing<W: Write>(
    out: &mut W,
    state: &State,
    idx: usize,
    buf: &[char],
    cursor_pos: usize,
) -> Result<()> {
    // Render with the edited token shown as the buffer contents (unquoted,
    // so the user sees exactly what they're typing).
    let mut line = String::new();
    let mut tok_start = 0usize;
    let mut tok_len = 0usize;
    for (i, t) in state.tokens.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        if i == idx {
            tok_start = line.chars().count();
            let s: String = buf.iter().collect();
            tok_len = s.chars().count();
            line.push_str(&s);
        } else {
            let r = if t.quoted || t.text.is_empty() {
                shell_words::quote(&t.text).into_owned()
            } else {
                t.text.clone()
            };
            line.push_str(&r);
        }
    }

    queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    queue!(
        out,
        SetAttribute(Attribute::Bold),
        Print("tweak command\r\n"),
        SetAttribute(Attribute::Reset)
    )?;
    queue!(out, cursor::MoveTo(CMD_COL, CMD_ROW))?;
    let before: String = line.chars().take(tok_start).collect();
    let tok: String = line.chars().skip(tok_start).take(tok_len).collect();
    let after: String = line.chars().skip(tok_start + tok_len).collect();
    queue!(
        out,
        Print(before),
        SetAttribute(Attribute::Underlined),
        SetForegroundColor(Color::Yellow),
        Print(&tok),
        ResetColor,
        SetAttribute(Attribute::Reset),
        Print(after),
    )?;

    write_status(out, &status_text(state))?;

    let cursor_col = CMD_COL + (tok_start + cursor_pos) as u16;
    queue!(out, cursor::MoveTo(cursor_col, CMD_ROW), cursor::Show)?;
    out.flush()?;
    Ok(())
}

fn status_text(state: &State) -> String {
    if let Some(s) = &state.status {
        return s.clone();
    }
    match &state.mode {
        Mode::Normal => DEFAULT_STATUS.into(),
        Mode::AwaitHint(HintOp::Delete) => "d — press a hint to delete (Esc to cancel)".into(),
        Mode::AwaitHint(HintOp::InsertBefore) => {
            "i — press a hint to insert before (Esc to cancel)".into()
        }
        Mode::AwaitHint(HintOp::InsertAfter) => {
            "a — press a hint to insert after (Esc to cancel)".into()
        }
        Mode::Editing { .. } => "editing · Enter commit · Esc cancel · Ctrl-U clear".into(),
    }
}

fn write_status<W: Write>(out: &mut W, text: &str) -> Result<()> {
    let (_, rows) = terminal::size()?;
    queue!(
        out,
        cursor::Hide,
        cursor::MoveTo(0, rows.saturating_sub(1)),
        Clear(ClearType::CurrentLine),
        SetAttribute(Attribute::Dim),
        Print(text),
        SetAttribute(Attribute::Reset),
    )?;
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
