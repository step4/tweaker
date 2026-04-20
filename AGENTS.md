# AGENTS.md

Guidance for AI coding agents working in this repo.

---

## What this project is

`tweaker` is a Rust TUI CLI that loads recent shell history, lets the user pick a command, then presents each token with an easymotion-style hint label. The user presses a hint to edit, delete, or insert tokens in place, then accepts to run the result.

**Key invariant:** the TUI renders to **stderr**. The final accepted command goes to **stdout**. Never break this — it is what makes `$(tweaker --print)` work correctly inside shell widgets.

---

## Module map

| File             | Responsibility                                                                                      |
| ---------------- | --------------------------------------------------------------------------------------------------- |
| `src/main.rs`    | CLI entry (`clap`), `init` subcommand (shell snippets), `spawn_in_shell` (cross-platform execution) |
| `src/history.rs` | Load + dedupe shell history; detect history file; strip zsh extended-history prefix                 |
| `src/tokens.rs`     | `split` / `render` / `render_with_spans`; hint label ↔ index mapping; `QuoteStyle` enum          |
| `src/state.rs`      | Pure state machine: `State`, `Mode`, `Action`, `Outcome`, undo/redo stacks                        |
| `src/suggestions.rs`| `load(cmd)` → tldr cache parser + man-page fallback; `Suggestion` / `SuggestionKind` types       |
| `src/tui.rs`        | Key → `Action` mapping; `ratatui` rendering for picker, tweak, and suggestions panel             |

---

## Architecture: state machine + thin TUI layer

`state.rs` is IO-free and fully unit-tested. `tui.rs` only:

1. Translates `crossterm::KeyEvent` → `Action` via `key_to_action`.
2. Calls `state.apply(action)` and matches the returned `Outcome`.
3. Renders from `State` using `ratatui`.

Do not add logic to `tui.rs`. If a new command needs new behaviour, add it to `state.rs` first and test it there, then wire the key in `key_to_action`.

### Modes

```
Normal
  ├── Hint(ch)       → Editing { idx, buf, cursor, inserted: false }
  ├── Prefix(Delete) → AwaitHint(Delete)
  ├── Prefix(InsertAfter) → AwaitHint(InsertAfter)
  ├── Prefix(InsertBefore) → AwaitHint(InsertBefore)
  ├── Undo / Redo    → Normal (tokens mutated, undo/redo stacks updated)
  ├── Commit         → Accept (Outcome)
  └── Cancel         → Quit (Outcome)

AwaitHint(op)
  ├── Hint(ch) [valid idx] → Editing (insert ops) or Normal (delete)
  └── Cancel / invalid    → Normal

Editing { idx, buf, cursor, inserted, quote_style }
  ├── Char / cursor keys  → Editing (buf mutated)
  ├── ToggleQuote         → Editing (quote_style cycled: None → Single → Double → None)
  ├── Commit              → Normal (tokens[idx].text = buf, re-quoted via quote_style.apply)
  └── Cancel              → Normal (buf discarded; token removed if inserted=true)

BrowsingSuggestions { selected }
  ├── SuggestionUp / SuggestionDown  → BrowsingSuggestions (selection wraps)
  ├── ApplySuggestion (Example)      → Normal (tokens replaced by split(example))
  ├── ApplySuggestion (Flag)         → Normal (flag appended as new token)
  └── Cancel / FocusSuggestions      → Normal
```

### Undo / redo

`tokens: Vec<Token>` is snapshotted before each `apply` call. If `tokens` changed after the call, the snapshot is pushed to `state.undo` and `state.redo` is cleared. `Undo`/`Redo` actions bypass this snapshot loop and swap directly. Only operates in `Mode::Normal`.

---

## Label alphabet

Labels run `1–9` then `A–Z` (uppercase). Lowercase letters are **reserved as command prefixes** (`d`, `a`, `i`, `u`). Do not assign new single-key commands using uppercase letters or digits — those are hint targets.

---

## Tokens and quoting

`tokens::split` is a custom POSIX tokeniser (no external shell-words dependency). It preserves each token's `original` source form (including quotes) alongside its logical `text` value, so `render` can reconstruct the command character-for-character without losing quoting.

`QuoteStyle` (`None` / `Single` / `Double`) controls how a token is re-serialised after an edit. The user cycles it with `Ctrl+S` during editing; `state.rs` calls `quote_style.apply(&text)` on commit. `QuoteStyle::None` falls back to minimal POSIX single-quoting (safe chars are left bare). **Never simplify `render` into a plain `join(" ")` — it silently corrupts commands with spaces or special characters.**

---

## Shell integration

Three shells supported. Each snippet is a `const &str` in `main.rs` emitted by `tweaker init <shell>`:

| Shell      | Keybind  | Mechanism                                               |
| ---------- | -------- | ------------------------------------------------------- |
| zsh        | `Ctrl+G` | ZLE widget, `BUFFER=...; zle accept-line`               |
| bash       | `Ctrl+G` | `bind -x`, `READLINE_LINE=...`                          |
| PowerShell | `Ctrl+G` | `Set-PSReadLineKeyHandler`, `PSConsoleReadLine::Insert` |

All widgets call `tweaker --print` (stdout = command, TUI on stderr) and push the result into the shell's readline buffer, so the executed command is recorded in history but the `tweaker` invocation is not.

---

## Windows

- **Terminal**: crossterm + ratatui handle Windows Terminal natively (VT processing enabled automatically).
- **History**: `history::detect()` checks `%APPDATA%\Microsoft\Windows\PowerShell\PSReadLine\ConsoleHost_history.txt` on Windows before the Unix dotfile fallbacks.
- **Execution**: `spawn_in_shell` on Windows tries `$SHELL -c` first (Git Bash), then falls back to `powershell.exe -NoProfile -Command`.
- **Shell integration**: use `tweaker init powershell`.

---

## Testing

```sh
cargo test
```

55 tests, no TTY required. Structure:

- `tokens::tests` — split/render roundtrip, quoting, label↔index mapping.
- `history::tests` — `parse_line` variants, `load` dedup + limit (uses `tempfile`).
- `state::tests` — full state machine: every mode transition, undo/redo, cursor movement, insert/cancel edge cases, suggestions browsing.
- `suggestions::tests` — tldr Markdown parser, man-page backspace stripping and OPTIONS extraction, cache dir resolution.

### Adding tests

- Logic in `tokens.rs` or `history.rs`: add `#[test]` in the same file.
- New state machine behaviour: add to `state::tests`. Drive it with `Action` sequences and assert on `rendered(&s)` + `s.mode`.
- **Do not** test rendered ANSI/ratatui output — it changes with every layout tweak.
- PTY/integration tests: use `expectrl` if truly needed; keep the set tiny and mark `#[ignore]` on CI if flaky.

---

## Keeping docs in sync

When you add or change a user-facing feature (new key binding, new CLI flag, new mode behaviour), update **README.md** in the same commit. Specifically:

- New key in edit mode → add a row to the "Editing a token" table.
- New key in normal mode → add a row to the "Normal mode keys" table.
- New CLI flag → update the CLI reference block.
- Changed history detection order → update the "History file detection" list.

Also update the module map, mode diagram, or test count in this file if the architecture changed.

---

## Before committing

**Always run all three before marking work done:**

```sh
cargo fmt
cargo clippy   # must be warning-free
cargo test
```

Clippy pitfalls that have bitten this codebase before:

- Methods named `from_*` must take no `self` (clippy::wrong_self_convention). Use `handle_*` or `process_*` for `&mut self` dispatch methods.
- Nested `if let` / `if let … && !x` blocks should be collapsed with `&&` let-chains.
- `loop { let Some(x) = … else { break }; … }` should be `while let Some(x) = … { … }`.
- `pub fn` items only used in `#[cfg(test)]` still trigger `dead_code`. Either remove the wrapper or inline the call in the test.

---

## Conventions

- `anyhow::Result` everywhere; use `?` freely. No custom error types.
- No `println!` except in `main` after the TUI exits (the final command echo). Everything interactive uses `stderr`.
- `TermGuard` in `tui.rs` is RAII: raw mode and alternate screen are always restored on drop, even on panic. Any new screen must be inside a `TermGuard` scope.
- Keep deps justified. Current set: `clap`, `crossterm`, `ratatui`, `anyhow`, `dirs`. Dev: `insta`, `tempfile`.

---

## Building

```sh
cargo build
cargo run -- --limit 100
cargo run -- --history-file /tmp/fake_history
cargo run -- init zsh
```

---

## Known limitations / future work

- **zsh metafied encoding**: zsh can write high-byte characters in a metafied encoding in some configs. `history.rs` does not decode this; non-ASCII entries may appear mangled.
- **Wide characters**: label column alignment assumes 1 char = 1 terminal cell. CJK/emoji tokens will misalign hint labels.
- **`state::tests` are headless only**: the TUI render path has no automated tests. A future PTY smoke test with `expectrl` would catch raw-mode leaks.
