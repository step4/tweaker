# AGENTS.md

Guidance for AI coding agents working in this repo.

## What this project is

`tweaker` is a small Rust CLI that loads recent shell history, lets the user
pick a previous command, then tokenizes it and shows a labeled view
(`[1] git`, `[2] commit`, `[3] -m`, `[4] "msg"` …). The user hits a label key
to edit that one token in place, then Enter to print the rewritten command to
stdout.

The intended usage is as a shell helper:

```sh
cmd=$(tweaker) && print -z -- "$cmd"   # zsh: put it on the prompt
```

UI is drawn to **stderr** so the final rewritten command on **stdout** is
pipe- and command-substitution-safe. Preserve this invariant.

## Layout

- `src/main.rs` — clap entrypoint, wires `history → tui::pick_entry → tui::tweak`.
- `src/history.rs` — reads `$HISTFILE` / `~/.zsh_history` / `~/.bash_history`.
  Handles zsh extended-history prefix (`: 1700000000:0;cmd`).
- `src/tokens.rs` — split/join via `shell-words`, plus the label ↔ index
  mapping (`1..9` then `a..z`).
- `src/tui.rs` — crossterm-based picker and tweak screens, with a
  `RawGuard` RAII type that enters/leaves the alternate screen.

## Conventions

- `anyhow::Result` everywhere; use `?` freely. No custom error types yet.
- All interactive UI writes go through `stderr()`. Never println! to stdout
  except the final rewritten command.
- When adding a new interactive screen, wrap it in `RawGuard::enter()` so
  raw mode is always restored on panic / early return.
- Keep external deps minimal — current set (`clap`, `crossterm`,
  `shell-words`, `anyhow`, `dirs`) is deliberate. Justify additions.

## Building / running

```sh
cargo build
cargo run -- --limit 100
cargo run -- --history-file /path/to/hist
```

There are no tests yet. If you add logic in `tokens.rs` (split/join
roundtrip, label mapping), add unit tests there — it's the easiest module
to test without a TTY.

## Things to be careful about

- `shell-words::split` strips quoting; `tokens::join` re-quotes when a token
  contains whitespace or shell metacharacters. Don't "simplify" this into a
  plain `join(" ")` — it silently corrupts commands with spaces or quotes.
- History files can be huge; we reverse + truncate to `--limit`. Don't load
  the whole file into memory in a form that scales worse than `Vec<String>`
  of recent entries.
- zsh writes history with a metafied encoding for high bytes in some
  configs. We currently don't decode it; if you touch `history.rs`, be
  aware that non-ASCII entries may look mangled and fixing that is a real
  task, not a one-liner.
