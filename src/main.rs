mod history;
mod tokens;
mod tui;

use anyhow::{Context, Result};
use clap::Parser;
use std::process::{Command, ExitCode};

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
    #[arg(long)]
    print: bool,
}

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
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

    // Echo what's about to run, then hand the terminal off to the user's shell.
    eprintln!("\x1b[2m$ {out}\x1b[0m");
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let status = Command::new(&shell)
        .arg("-c")
        .arg(&out)
        .status()
        .with_context(|| format!("spawning {shell}"))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}
