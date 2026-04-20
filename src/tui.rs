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
    let mut tokens = tokens::split(cmd)?;
    let _g = RawGuard::enter()?;
    let mut out = stderr();
    let mut status = String::from(DEFAULT_STATUS);
    let mut status_expires: Option<Instant> = None;
    loop {
        draw_hints(&mut out, &tokens, &status)?;

        let k = match status_expires {
            Some(deadline) => match read_key_until(deadline)? {
                Some(k) => k,
                None => {
                    status = DEFAULT_STATUS.into();
                    status_expires = None;
                    continue;
                }
            },
            None => read_key()?,
        };

        let mut transient: Option<String> = None;
        let mut quit = false;
        let mut accept = false;

        match k.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => quit = true,
            KeyCode::Enter => accept = true,
            KeyCode::Char('d') => {
                status = "d — press a hint to delete (Esc to cancel)".into();
                status_expires = None;
                draw_hints(&mut out, &tokens, &status)?;
                transient = Some(match read_hint(tokens.len())? {
                    Some(idx) => {
                        tokens.remove(idx);
                        format!("deleted token {}", idx + 1)
                    }
                    None => "delete cancelled".into(),
                });
            }
            KeyCode::Char('a') | KeyCode::Char('i') => {
                let before = matches!(k.code, KeyCode::Char('i'));
                let (word, label) = if before {
                    ("before", "i")
                } else {
                    ("after", "a")
                };
                status = format!("{label} — press a hint to insert {word} (Esc to cancel)");
                status_expires = None;
                draw_hints(&mut out, &tokens, &status)?;
                transient = Some(match read_hint(tokens.len())? {
                    Some(idx) => {
                        let new_idx = if before { idx } else { idx + 1 };
                        tokens.insert(
                            new_idx,
                            tokens::Token {
                                text: String::new(),
                                quoted: false,
                            },
                        );
                        let committed = edit_inline(&mut out, &mut tokens, new_idx)?;
                        if !committed || tokens[new_idx].text.is_empty() {
                            tokens.remove(new_idx);
                            "insert cancelled".into()
                        } else {
                            format!("inserted {word} token {}", idx + 1)
                        }
                    }
                    None => "insert cancelled".into(),
                });
            }
            KeyCode::Char(ch) => {
                if let Some(idx) = tokens::index_for(ch) {
                    transient = Some(if idx < tokens.len() {
                        if edit_inline(&mut out, &mut tokens, idx)? {
                            format!("edited [{}]", ch)
                        } else {
                            format!("cancelled [{}]", ch)
                        }
                    } else {
                        format!("no token at [{}]", ch)
                    });
                }
            }
            _ => {}
        }

        if quit {
            return Ok(None);
        }
        if accept {
            return Ok(Some(tokens::render(&tokens)));
        }
        if let Some(msg) = transient {
            status = msg;
            status_expires = Some(Instant::now() + STATUS_TTL);
        }
    }
}

/// Read one keystroke as a hint; returns None on Esc or an unknown key.
fn read_hint(n_tokens: usize) -> Result<Option<usize>> {
    let k = read_key()?;
    match k.code {
        KeyCode::Esc => Ok(None),
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Ok(None),
        KeyCode::Char(ch) => match tokens::index_for(ch) {
            Some(i) if i < n_tokens => Ok(Some(i)),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn draw_hints<W: Write>(out: &mut W, tokens: &[tokens::Token], status: &str) -> Result<()> {
    let (rendered, spans) = tokens::render_with_spans(tokens);
    queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    queue!(
        out,
        SetAttribute(Attribute::Bold),
        Print("tweak command\r\n"),
        SetAttribute(Attribute::Reset)
    )?;

    // Command on its own line.
    queue!(out, cursor::MoveTo(CMD_COL, CMD_ROW), Print(&rendered))?;

    // Labels on the row below, aligned to each token's first column.
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

    let (_, rows) = terminal::size()?;
    queue!(
        out,
        cursor::MoveTo(0, rows.saturating_sub(1)),
        Clear(ClearType::CurrentLine),
        SetAttribute(Attribute::Dim),
        Print(status),
        SetAttribute(Attribute::Reset),
    )?;
    out.flush()?;
    Ok(())
}

/// Edit token `idx` in place at its actual column in the rendered command.
/// Returns Ok(true) on commit, Ok(false) on cancel.
fn edit_inline<W: Write>(out: &mut W, tokens: &mut Vec<tokens::Token>, idx: usize) -> Result<bool> {
    let original = tokens[idx].clone();
    let mut buf: Vec<char> = tokens[idx].text.chars().collect();
    let mut cursor_pos = buf.len();

    loop {
        // Reflect current buffer into the token so render_with_spans gives us real columns.
        tokens[idx].text = buf.iter().collect();
        tokens[idx].quoted = needs_quote(&tokens[idx].text);
        let (rendered, spans) = tokens::render_with_spans(tokens);
        let (tok_start, tok_len) = spans[idx];

        queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        queue!(
            out,
            SetAttribute(Attribute::Bold),
            Print("tweak command\r\n"),
            SetAttribute(Attribute::Reset)
        )?;
        // Print everything before the edited token plainly.
        queue!(out, cursor::MoveTo(CMD_COL, CMD_ROW))?;
        let before: String = rendered.chars().take(tok_start).collect();
        let token_str: String = rendered.chars().skip(tok_start).take(tok_len).collect();
        let after: String = rendered.chars().skip(tok_start + tok_len).collect();
        queue!(
            out,
            Print(before),
            SetAttribute(Attribute::Underlined),
            SetForegroundColor(Color::Yellow),
            Print(&token_str),
            ResetColor,
            SetAttribute(Attribute::Reset),
            Print(after),
        )?;

        // Status line.
        let (_, rows) = terminal::size()?;
        queue!(
            out,
            cursor::MoveTo(0, rows.saturating_sub(1)),
            Clear(ClearType::CurrentLine),
            SetAttribute(Attribute::Dim),
            Print("editing · Enter commit · Esc cancel · Ctrl-U clear"),
            SetAttribute(Attribute::Reset),
        )?;

        // Compute cursor column inside the rendered token. If the token is quoted,
        // there's an opening quote char before the buffer content — offset by that.
        let quote_prefix = if tokens[idx].quoted { 1 } else { 0 };
        let cursor_col = CMD_COL + (tok_start + quote_prefix + cursor_pos) as u16;
        queue!(out, cursor::MoveTo(cursor_col, CMD_ROW), cursor::Show)?;
        out.flush()?;

        let k = read_key()?;
        match k.code {
            KeyCode::Esc => {
                tokens[idx] = original;
                queue!(out, cursor::Hide)?;
                return Ok(false);
            }
            KeyCode::Enter => {
                queue!(out, cursor::Hide)?;
                return Ok(true);
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                buf.clear();
                cursor_pos = 0;
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                tokens[idx] = original;
                queue!(out, cursor::Hide)?;
                return Ok(false);
            }
            KeyCode::Char(c) => {
                buf.insert(cursor_pos, c);
                cursor_pos += 1;
            }
            KeyCode::Backspace => {
                if cursor_pos > 0 {
                    cursor_pos -= 1;
                    buf.remove(cursor_pos);
                }
            }
            KeyCode::Delete => {
                if cursor_pos < buf.len() {
                    buf.remove(cursor_pos);
                }
            }
            KeyCode::Left => cursor_pos = cursor_pos.saturating_sub(1),
            KeyCode::Right => {
                if cursor_pos < buf.len() {
                    cursor_pos += 1;
                }
            }
            KeyCode::Home => cursor_pos = 0,
            KeyCode::End => cursor_pos = buf.len(),
            _ => {}
        }
    }
}

fn needs_quote(s: &str) -> bool {
    s.is_empty() || s.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c))
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
