mod history;
mod state;
mod tokens;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::process::{Command, ExitCode, ExitStatus};

#[derive(Parser)]
#[command(
    name = "tweaker",
    about = "Pick a previous shell command and tweak its flags/args interactively."
)]
struct Cli {
    /// Max history entries to load.
    #[arg(long, default_value_t = 200)]
    limit: usize,

    /// Path to a history file. Auto-detected from $HISTFILE / shell default if omitted.
    #[arg(long)]
    history_file: Option<std::path::PathBuf>,

    /// Print the tweaked command to stdout instead of executing it.
    /// Shell widgets use this; direct CLI use does not.
    #[arg(long)]
    print: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print shell integration snippet. Source via: eval "$(tweaker init zsh)"
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum Shell {
    Zsh,
    Bash,
    Powershell,
}

const ZSH_INIT: &str = r##"# tweaker zsh integration — source via: eval "$(tweaker init zsh)"
tweaker-widget() {
  local cmd
  cmd=$(tweaker --print </dev/tty) || return
  [[ -z $cmd ]] && { zle reset-prompt; return }
  BUFFER=$cmd
  CURSOR=${#BUFFER}
  zle accept-line
}
zle -N tweaker-widget
bindkey '^G' tweaker-widget
"##;

const BASH_INIT: &str = r##"# tweaker bash integration — source via: eval "$(tweaker init bash)"
__tweaker_widget() {
  local cmd
  cmd=$(tweaker --print </dev/tty) || return
  [[ -z $cmd ]] && return
  READLINE_LINE=$cmd
  READLINE_POINT=${#cmd}
}
bind -x '"\C-g": __tweaker_widget'
"##;

const POWERSHELL_INIT: &str = r##"# tweaker PowerShell integration — add to $PROFILE:
#   Add-Content $PROFILE "$(tweaker init powershell)"
function Invoke-Tweaker {
    # TUI renders to stderr; stdout carries the final command.
    $cmd = tweaker --print
    if ($cmd) {
        [Microsoft.PowerShell.PSConsoleReadLine]::Insert($cmd)
    }
}
Set-PSReadLineKeyHandler -Key 'Ctrl+g' -ScriptBlock { Invoke-Tweaker }
"##;

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();

    if let Some(Cmd::Init { shell }) = cli.cmd {
        let snippet = match shell {
            Shell::Zsh => ZSH_INIT,
            Shell::Bash => BASH_INIT,
            Shell::Powershell => POWERSHELL_INIT,
        };
        print!("{snippet}");
        return Ok(ExitCode::SUCCESS);
    }

    let entries = history::load(cli.history_file.as_deref(), cli.limit)?;
    if entries.is_empty() {
        anyhow::bail!("no history entries found");
    }
    let Some(cmd) = tui::pick_entry(&entries)? else {
        return Ok(ExitCode::SUCCESS);
    };
    let Some(out) = tui::tweak(&cmd)? else {
        return Ok(ExitCode::SUCCESS);
    };

    if cli.print {
        println!("{out}");
        return Ok(ExitCode::SUCCESS);
    }

    eprintln!("\x1b[2m$ {out}\x1b[0m");
    let status = spawn_in_shell(&out)?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn spawn_in_shell(cmd: &str) -> Result<ExitStatus> {
    #[cfg(windows)]
    {
        // Prefer $SHELL (Git Bash / WSL interop), fall back to PowerShell.
        if let Ok(shell) = std::env::var("SHELL") {
            if let Ok(s) = Command::new(&shell).arg("-c").arg(cmd).status() {
                return Ok(s);
            }
        }
        Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", cmd])
            .status()
            .context("spawning powershell.exe")
    }

    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        Command::new(&shell)
            .arg("-c")
            .arg(cmd)
            .status()
            .with_context(|| format!("spawning {shell}"))
    }
}
