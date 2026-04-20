use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub fn load(explicit: Option<&Path>, limit: usize) -> Result<Vec<String>> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => detect()?,
    };
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("reading history file {}", path.display()))?;

    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let cmd = parse_line(line);
        if cmd.is_empty() {
            continue;
        }
        // Collapse consecutive duplicates.
        if out.last().map(|s| s.as_str()) == Some(cmd) {
            continue;
        }
        out.push(cmd.to_string());
    }
    // Most recent last; reverse so newest is index 0.
    out.reverse();
    out.truncate(limit);
    Ok(out)
}

/// Strip the zsh extended-history prefix ": 1234567890:0;" if present.
fn parse_line(line: &str) -> &str {
    let trimmed = line.trim_end_matches('\\').trim();
    if let Some(rest) = trimmed.strip_prefix(':') {
        if let Some(idx) = rest.find(';') {
            return rest[idx + 1..].trim();
        }
    }
    trimmed
}

fn detect() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("HISTFILE") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let home = dirs::home_dir().context("no home dir")?;
    for name in [".zsh_history", ".bash_history", ".history"] {
        let p = home.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    anyhow::bail!("could not find a shell history file; pass --history-file")
}
