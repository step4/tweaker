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

pub fn tweak(cmd: &str) -> Result<Option<String>> {
    let mut tokens = tokens::split(cmd)?;
    let _g = RawGuard::enter()?;
    let mut out = stderr();
    let mut status = String::from("press a label to edit that token, Enter to accept, Esc to cancel");
    loop {
        queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        queue!(
            out,
            SetAttribute(Attribute::Bold),
            Print("tweak command\r\n"),
            SetAttribute(Attribute::Reset)
        )?;

        // Render tokens with labels above them.
        let rendered = tokens::join(&tokens);
        queue!(out, Print(format!("\r\n  {}\r\n\r\n", rendered)))?;

        for (i, t) in tokens.iter().enumerate() {
            let Some(lbl) = tokens::label(i) else { break };
            queue!(
                out,
                SetForegroundColor(Color::Yellow),
                Print(format!("  [{}]", lbl)),
                ResetColor,
                Print(format!(" {}\r\n", t.text))
            )?;
        }
        queue!(out, Print(format!("\r\n{}\r\n", status)))?;
        out.flush()?;

        let k = read_key()?;
        match k.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(None),
            KeyCode::Enter => return Ok(Some(tokens::join(&tokens))),
            KeyCode::Char(ch) => {
                if let Some(idx) = tokens::index_for(ch) {
                    if idx < tokens.len() {
                        if let Some(new_val) = edit_prompt(&tokens[idx].text)? {
                            tokens[idx].text = new_val;
                            tokens[idx].quoted = tokens[idx]
                                .text
                                .chars()
                                .any(|c| c.is_whitespace() || "\"'\\$`".contains(c));
                            status = format!("edited [{}]", ch);
                        }
                    } else {
                        status = format!("no token at [{}]", ch);
                    }
                }
            }
            _ => {}
        }
    }
}

fn edit_prompt(initial: &str) -> Result<Option<String>> {
    // Simple line editor on the bottom row.
    let mut buf: Vec<char> = initial.chars().collect();
    let mut cursor_pos = buf.len();
    let mut out = stderr();
    loop {
        let (_, rows) = terminal::size()?;
        let row = rows.saturating_sub(1);
        queue!(
            out,
            cursor::MoveTo(0, row),
            Clear(ClearType::CurrentLine),
            cursor::Show,
            Print("edit> "),
            Print(buf.iter().collect::<String>()),
            cursor::MoveTo((6 + cursor_pos) as u16, row),
        )?;
        out.flush()?;

        let k = read_key()?;
        match k.code {
            KeyCode::Esc => {
                queue!(out, cursor::Hide)?;
                return Ok(None);
            }
            KeyCode::Enter => {
                queue!(out, cursor::Hide)?;
                return Ok(Some(buf.into_iter().collect()));
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                buf.clear();
                cursor_pos = 0;
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                queue!(out, cursor::Hide)?;
                return Ok(None);
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
