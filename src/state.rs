//! Pure state machine for the tweak screen. No IO, no terminal — the TUI
//! layer translates key events into `Action`s and renders from `State`.

use crate::tokens::{self, Token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintOp {
    Delete,
    InsertBefore,
    InsertAfter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    AwaitHint(HintOp),
    Editing {
        idx: usize,
        buf: Vec<char>,
        cursor: usize,
        /// True if the token was freshly inserted for this edit session;
        /// on cancel or empty-commit we remove it instead of keeping empty text.
        inserted: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Label keystroke (1-9, A-Z). In Normal mode this starts an edit; in
    /// AwaitHint it picks the target token.
    Hint(char),
    Prefix(HintOp),
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    ClearLine,
    Commit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Accept,
    Quit,
}

#[derive(Debug, Clone)]
pub struct State {
    pub tokens: Vec<Token>,
    pub mode: Mode,
    pub status: Option<String>,
}

impl State {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            mode: Mode::Normal,
            status: None,
        }
    }

    pub fn apply(&mut self, action: Action) -> Outcome {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        let (new_mode, outcome) = match mode {
            Mode::Normal => self.from_normal(action),
            Mode::AwaitHint(op) => self.from_await(op, action),
            Mode::Editing {
                idx,
                buf,
                cursor,
                inserted,
            } => self.from_editing(idx, buf, cursor, inserted, action),
        };
        self.mode = new_mode;
        outcome
    }

    fn from_normal(&mut self, action: Action) -> (Mode, Outcome) {
        match action {
            Action::Cancel => (Mode::Normal, Outcome::Quit),
            Action::Commit => (Mode::Normal, Outcome::Accept),
            Action::Prefix(op) => (Mode::AwaitHint(op), Outcome::Continue),
            Action::Hint(ch) => match resolve_hint(ch, self.tokens.len()) {
                Some(idx) => {
                    let buf: Vec<char> = self.tokens[idx].text.chars().collect();
                    let cursor = buf.len();
                    (
                        Mode::Editing {
                            idx,
                            buf,
                            cursor,
                            inserted: false,
                        },
                        Outcome::Continue,
                    )
                }
                None => {
                    self.status = Some(format!("no token at [{ch}]"));
                    (Mode::Normal, Outcome::Continue)
                }
            },
            _ => (Mode::Normal, Outcome::Continue),
        }
    }

    fn from_await(&mut self, op: HintOp, action: Action) -> (Mode, Outcome) {
        match action {
            Action::Cancel => {
                self.status = Some("cancelled".into());
                (Mode::Normal, Outcome::Continue)
            }
            Action::Hint(ch) => match resolve_hint(ch, self.tokens.len()) {
                Some(idx) => match op {
                    HintOp::Delete => {
                        self.tokens.remove(idx);
                        self.status = Some(format!("deleted token {}", idx + 1));
                        (Mode::Normal, Outcome::Continue)
                    }
                    HintOp::InsertBefore | HintOp::InsertAfter => {
                        let new_idx = if op == HintOp::InsertBefore {
                            idx
                        } else {
                            idx + 1
                        };
                        self.tokens.insert(
                            new_idx,
                            Token {
                                text: String::new(),
                                quoted: false,
                            },
                        );
                        (
                            Mode::Editing {
                                idx: new_idx,
                                buf: Vec::new(),
                                cursor: 0,
                                inserted: true,
                            },
                            Outcome::Continue,
                        )
                    }
                },
                None => {
                    self.status = Some(format!("no token at [{ch}]"));
                    (Mode::Normal, Outcome::Continue)
                }
            },
            // Unrelated keys while awaiting a hint are ignored.
            _ => (Mode::AwaitHint(op), Outcome::Continue),
        }
    }

    fn from_editing(
        &mut self,
        idx: usize,
        mut buf: Vec<char>,
        mut cursor: usize,
        inserted: bool,
        action: Action,
    ) -> (Mode, Outcome) {
        let keep = |buf, cursor| Mode::Editing {
            idx,
            buf,
            cursor,
            inserted,
        };
        match action {
            Action::Cancel => {
                if inserted {
                    self.tokens.remove(idx);
                }
                self.status = Some("cancelled".into());
                (Mode::Normal, Outcome::Continue)
            }
            Action::Commit => {
                let text: String = buf.iter().collect();
                if inserted && text.is_empty() {
                    self.tokens.remove(idx);
                    self.status = Some("insert cancelled".into());
                } else {
                    self.tokens[idx].text = text;
                    self.tokens[idx].quoted = tokens::needs_quote(&self.tokens[idx].text);
                    self.status = Some(format!("edited token {}", idx + 1));
                }
                (Mode::Normal, Outcome::Continue)
            }
            Action::Char(c) => {
                buf.insert(cursor, c);
                cursor += 1;
                (keep(buf, cursor), Outcome::Continue)
            }
            Action::Backspace => {
                if cursor > 0 {
                    cursor -= 1;
                    buf.remove(cursor);
                }
                (keep(buf, cursor), Outcome::Continue)
            }
            Action::Delete => {
                if cursor < buf.len() {
                    buf.remove(cursor);
                }
                (keep(buf, cursor), Outcome::Continue)
            }
            Action::Left => {
                cursor = cursor.saturating_sub(1);
                (keep(buf, cursor), Outcome::Continue)
            }
            Action::Right => {
                if cursor < buf.len() {
                    cursor += 1;
                }
                (keep(buf, cursor), Outcome::Continue)
            }
            Action::Home => (keep(buf, 0), Outcome::Continue),
            Action::End => {
                let c = buf.len();
                (keep(buf, c), Outcome::Continue)
            }
            Action::ClearLine => (keep(Vec::new(), 0), Outcome::Continue),
            Action::Hint(_) | Action::Prefix(_) => (keep(buf, cursor), Outcome::Continue),
        }
    }
}

fn resolve_hint(ch: char, n: usize) -> Option<usize> {
    tokens::index_for(ch).filter(|&i| i < n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens;

    fn state(cmd: &str) -> State {
        State::new(tokens::split(cmd).unwrap())
    }

    fn run(s: &mut State, actions: impl IntoIterator<Item = Action>) -> Outcome {
        let mut last = Outcome::Continue;
        for a in actions {
            last = s.apply(a);
            if !matches!(last, Outcome::Continue) {
                return last;
            }
        }
        last
    }

    fn rendered(s: &State) -> String {
        tokens::render(&s.tokens)
    }

    #[test]
    fn enter_in_normal_accepts() {
        let mut s = state("ls -la");
        assert_eq!(s.apply(Action::Commit), Outcome::Accept);
    }

    #[test]
    fn esc_in_normal_quits() {
        let mut s = state("ls -la");
        assert_eq!(s.apply(Action::Cancel), Outcome::Quit);
    }

    #[test]
    fn edit_token_by_hint() {
        let mut s = state("git commit -m hi");
        s.apply(Action::Hint('4'));
        assert!(matches!(s.mode, Mode::Editing { idx: 3, .. }));
        s.apply(Action::ClearLine);
        run(&mut s, "bye".chars().map(Action::Char));
        s.apply(Action::Commit);
        assert_eq!(s.mode, Mode::Normal);
        assert_eq!(rendered(&s), "git commit -m bye");
    }

    #[test]
    fn delete_removes_token() {
        let mut s = state("ls -la /tmp");
        s.apply(Action::Prefix(HintOp::Delete));
        assert!(matches!(s.mode, Mode::AwaitHint(HintOp::Delete)));
        s.apply(Action::Hint('2'));
        assert_eq!(rendered(&s), "ls /tmp");
    }

    #[test]
    fn insert_after_adds_token() {
        let mut s = state("ls /tmp");
        s.apply(Action::Prefix(HintOp::InsertAfter));
        s.apply(Action::Hint('1'));
        run(&mut s, "-la".chars().map(Action::Char));
        s.apply(Action::Commit);
        assert_eq!(rendered(&s), "ls -la /tmp");
    }

    #[test]
    fn insert_before_adds_token() {
        let mut s = state("ls /tmp");
        s.apply(Action::Prefix(HintOp::InsertBefore));
        s.apply(Action::Hint('2'));
        run(&mut s, "/home".chars().map(Action::Char));
        s.apply(Action::Commit);
        assert_eq!(rendered(&s), "ls /home /tmp");
    }

    #[test]
    fn empty_insert_commit_removes_placeholder() {
        let mut s = state("ls /tmp");
        s.apply(Action::Prefix(HintOp::InsertAfter));
        s.apply(Action::Hint('1'));
        s.apply(Action::Commit);
        assert_eq!(rendered(&s), "ls /tmp");
    }

    #[test]
    fn cancel_edit_preserves_original_text() {
        let mut s = state("echo hi");
        s.apply(Action::Hint('2'));
        s.apply(Action::ClearLine);
        run(&mut s, "bye".chars().map(Action::Char));
        s.apply(Action::Cancel);
        assert_eq!(rendered(&s), "echo hi");
        assert_eq!(s.mode, Mode::Normal);
    }

    #[test]
    fn cancel_inserted_token_removes_it() {
        let mut s = state("ls");
        s.apply(Action::Prefix(HintOp::InsertAfter));
        s.apply(Action::Hint('1'));
        run(&mut s, "zz".chars().map(Action::Char));
        s.apply(Action::Cancel);
        assert_eq!(rendered(&s), "ls");
    }

    #[test]
    fn cursor_movement_and_home_insert() {
        let mut s = state("echo world");
        s.apply(Action::Hint('2'));
        s.apply(Action::Home);
        run(&mut s, "hi-".chars().map(Action::Char));
        s.apply(Action::Commit);
        assert_eq!(rendered(&s), "echo hi-world");
    }

    #[test]
    fn backspace_and_delete_in_edit() {
        let mut s = state("echo abcd");
        s.apply(Action::Hint('2'));
        s.apply(Action::Backspace); // abc
        s.apply(Action::Left);
        s.apply(Action::Left);
        s.apply(Action::Delete); // ac
        s.apply(Action::Commit);
        assert_eq!(rendered(&s), "echo ac");
    }

    #[test]
    fn unknown_hint_in_await_returns_to_normal() {
        let mut s = state("ls");
        s.apply(Action::Prefix(HintOp::Delete));
        s.apply(Action::Hint('9'));
        assert_eq!(s.mode, Mode::Normal);
        assert_eq!(rendered(&s), "ls");
    }

    #[test]
    fn cancel_in_await_returns_to_normal() {
        let mut s = state("ls");
        s.apply(Action::Prefix(HintOp::Delete));
        s.apply(Action::Cancel);
        assert_eq!(s.mode, Mode::Normal);
        assert_eq!(rendered(&s), "ls");
    }

    #[test]
    fn edited_token_with_space_gets_requoted() {
        let mut s = state("echo hi");
        s.apply(Action::Hint('2'));
        s.apply(Action::ClearLine);
        run(&mut s, "hi there".chars().map(Action::Char));
        s.apply(Action::Commit);
        // Whatever quoting shell_words picks, the rendered string must re-parse
        // into the same logical tokens.
        let reparsed = tokens::split(&tokens::render(&s.tokens)).unwrap();
        assert_eq!(reparsed[1].text, "hi there");
    }
}
