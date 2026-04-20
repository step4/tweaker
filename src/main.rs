mod history;
mod tokens;
mod tui;

use anyhow::Result;
use clap::Parser;

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let entries = history::load(cli.history_file.as_deref(), cli.limit)?;
    if entries.is_empty() {
        anyhow::bail!("no history entries found");
    }
    let Some(cmd) = tui::pick_entry(&entries)? else {
        return Ok(());
    };
    if let Some(out) = tui::tweak(&cmd)? {
        println!("{out}");
    }
    Ok(())
}
