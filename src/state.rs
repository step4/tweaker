//! Pure state machine for the tweak screen. No IO, no terminal — the TUI
//! layer translates key events into `Action`s and renders from `State`.

use crate::tokens::{self, QuoteStyle, Token};

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
        quote_style: QuoteStyle,
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
    Undo,
    Redo,
    ToggleQuote,
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
    undo: Vec<Vec<Token>>,
    redo: Vec<Vec<Token>>,
}

impl State {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            mode: Mode::Normal,
            status: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn apply(&mut self, action: Action) -> Outcome {
        // Undo/Redo short-circuit — they only operate in Normal mode and
        // bypass the snapshot machinery.
        match action {
            Action::Undo => return self.undo(),
            Action::Redo => return self.redo(),
            _ => {}
        }

        let snapshot = self.tokens.clone();
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        let (new_mode, outcome) = match mode {
            Mode::Normal => self.from_normal(action),
            Mode::AwaitHint(op) => self.from_await(op, action),
            Mode::Editing {
                idx,
                buf,
                cursor,
                inserted,
                quote_style,
            } => self.from_editing(idx, buf, cursor, inserted, quote_style, action),
        };
        self.mode = new_mode;

        // Any action that actually mutated tokens becomes an undo checkpoint.
        // Intermediate keystrokes inside Mode::Editing don't change self.tokens
        // (the buffer is mode-local), so only Commit / Delete / insert-removal
        // register as undoable events — which matches user intuition.
        if self.tokens != snapshot {
            self.undo.push(snapshot);
            self.redo.clear();
        }
        outcome
    }

    fn undo(&mut self) -> Outcome {
        if !matches!(self.mode, Mode::Normal) {
            return Outcome::Continue;
        }
        match self.undo.pop() {
            Some(prev) => {
                let current = std::mem::replace(&mut self.tokens, prev);
                self.redo.push(current);
                self.status = Some("undone".into());
            }
            None => {
                self.status = Some("nothing to undo".into());
            }
        }
        Outcome::Continue
    }

    fn redo(&mut self) -> Outcome {
        if !matches!(self.mode, Mode::Normal) {
            return Outcome::Continue;
        }
        match self.redo.pop() {
            Some(next) => {
                let current = std::mem::replace(&mut self.tokens, next);
                self.undo.push(current);
                self.status = Some("redone".into());
            }
            None => {
                self.status = Some("nothing to redo".into());
            }
        }
        Outcome::Continue
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
                    let quote_style = QuoteStyle::from_original(&self.tokens[idx].original);
                    (
                        Mode::Editing {
                            idx,
                            buf,
                            cursor,
                            inserted: false,
                            quote_style,
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
                                original: String::new(),
                            },
                        );
                        (
                            Mode::Editing {
                                idx: new_idx,
                                buf: Vec::new(),
                                cursor: 0,
                                inserted: true,
                                quote_style: QuoteStyle::None,
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
        quote_style: QuoteStyle,
        action: Action,
    ) -> (Mode, Outcome) {
        let keep = |buf: Vec<char>, cursor: usize| Mode::Editing {
            idx,
            buf,
            cursor,
            inserted,
            quote_style,
        };
        let keep_qs = |buf: Vec<char>, cursor: usize, qs: QuoteStyle| Mode::Editing {
            idx,
            buf,
            cursor,
            inserted,
            quote_style: qs,
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
                    self.tokens[idx].original = quote_style.apply(&self.tokens[idx].text);
                    self.status = Some(format!("edited token {}", idx + 1));
                }
                (Mode::Normal, Outcome::Continue)
            }
            Action::ToggleQuote => {
                (keep_qs(buf, cursor, quote_style.cycle()), Outcome::Continue)
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
            Action::Hint(_) | Action::Prefix(_) | Action::Undo | Action::Redo => {
                (keep(buf, cursor), Outcome::Continue)
            }
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
    fn undo_reverts_delete() {
        let mut s = state("ls -la /tmp");
        s.apply(Action::Prefix(HintOp::Delete));
        s.apply(Action::Hint('2'));
        assert_eq!(rendered(&s), "ls /tmp");
        s.apply(Action::Undo);
        assert_eq!(rendered(&s), "ls -la /tmp");
        assert_eq!(s.status.as_deref(), Some("undone"));
    }

    #[test]
    fn redo_reapplies_undone_change() {
        let mut s = state("ls -la /tmp");
        s.apply(Action::Prefix(HintOp::Delete));
        s.apply(Action::Hint('2'));
        s.apply(Action::Undo);
        s.apply(Action::Redo);
        assert_eq!(rendered(&s), "ls /tmp");
    }

    #[test]
    fn new_mutation_after_undo_clears_redo() {
        let mut s = state("ls -la /tmp");
        s.apply(Action::Prefix(HintOp::Delete));
        s.apply(Action::Hint('2')); // delete -la
        s.apply(Action::Undo); // back to "ls -la /tmp"
        // Now do a different mutation.
        s.apply(Action::Prefix(HintOp::Delete));
        s.apply(Action::Hint('3')); // delete /tmp
        assert_eq!(rendered(&s), "ls -la");
        // Redo of the old branch must be gone.
        s.apply(Action::Redo);
        assert_eq!(s.status.as_deref(), Some("nothing to redo"));
        assert_eq!(rendered(&s), "ls -la");
    }

    #[test]
    fn undo_with_nothing_reports_status() {
        let mut s = state("ls");
        s.apply(Action::Undo);
        assert_eq!(s.status.as_deref(), Some("nothing to undo"));
        assert_eq!(rendered(&s), "ls");
    }

    #[test]
    fn edit_commit_is_one_undo_step() {
        let mut s = state("echo hi");
        s.apply(Action::Hint('2'));
        s.apply(Action::ClearLine);
        run(&mut s, "bye".chars().map(Action::Char));
        s.apply(Action::Commit);
        assert_eq!(rendered(&s), "echo bye");
        // A single undo should restore the pre-edit text — intermediate char
        // keystrokes must not each be their own undo step.
        s.apply(Action::Undo);
        assert_eq!(rendered(&s), "echo hi");
    }

    #[test]
    fn undo_ignored_while_editing() {
        let mut s = state("echo hi");
        s.apply(Action::Hint('2'));
        s.apply(Action::Undo); // ignored: still Editing
        assert!(matches!(s.mode, Mode::Editing { .. }));
    }

    #[test]
    fn edit_preserves_single_quote_style() {
        let mut s = state("bindkey '^g' tweaker-widget");
        s.apply(Action::Hint('2')); // enters editing with QuoteStyle::Single
        s.apply(Action::Char('d'));
        s.apply(Action::Char('d'));
        s.apply(Action::Commit);
        assert_eq!(tokens::render(&s.tokens), "bindkey '^gdd' tweaker-widget");
    }

    #[test]
    fn toggle_quote_single_to_double() {
        let mut s = state("echo foo");
        s.apply(Action::Hint('2')); // QuoteStyle::None
        s.apply(Action::ToggleQuote); // → Single
        s.apply(Action::ToggleQuote); // → Double
        s.apply(Action::Commit);
        assert_eq!(tokens::render(&s.tokens), r#"echo "foo""#);
    }

    #[test]
    fn toggle_quote_wraps_on_commit() {
        let mut s = state("echo foo");
        s.apply(Action::Hint('2'));
        s.apply(Action::ToggleQuote); // → Single
        s.apply(Action::Commit);
        assert_eq!(tokens::render(&s.tokens), "echo 'foo'");
        // Re-parse: logical value unchanged.
        let reparsed = tokens::split(&tokens::render(&s.tokens)).unwrap();
        assert_eq!(reparsed[1].text, "foo");
        assert_eq!(reparsed[1].original, "'foo'");
    }

    #[test]
    fn toggle_quote_cycle_back_to_none() {
        let mut s = state("echo foo");
        s.apply(Action::Hint('2'));
        s.apply(Action::ToggleQuote); // → Single
        s.apply(Action::ToggleQuote); // → Double
        s.apply(Action::ToggleQuote); // → None
        s.apply(Action::Commit);
        // QuoteStyle::None uses shell_words::quote which leaves "foo" bare.
        assert_eq!(tokens::render(&s.tokens), "echo foo");
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
