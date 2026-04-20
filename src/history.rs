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
pub(crate) fn parse_line(line: &str) -> &str {
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

    #[cfg(windows)]
    if let Some(data) = dirs::data_dir() {
        // PSReadLine (PowerShell 5+) persistent history.
        let ps = data.join(r"Microsoft\Windows\PowerShell\PSReadLine\ConsoleHost_history.txt");
        if ps.exists() {
            return Ok(ps);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_plain_line() {
        assert_eq!(parse_line("ls -la"), "ls -la");
    }

    #[test]
    fn parse_zsh_extended_prefix() {
        assert_eq!(parse_line(": 1700000000:0;git status"), "git status");
    }

    #[test]
    fn parse_zsh_extended_with_whitespace() {
        assert_eq!(parse_line(": 1700000000:0;  cargo build  "), "cargo build");
    }

    #[test]
    fn parse_line_malformed_prefix_returned_verbatim() {
        // A leading colon with no semicolon is not the zsh extended format.
        assert_eq!(parse_line(":no-semi-here"), ":no-semi-here");
    }

    #[test]
    fn load_dedupes_consecutive_and_reverses() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "ls").unwrap();
        writeln!(f, "ls").unwrap(); // duplicate of previous
        writeln!(f, "pwd").unwrap();
        writeln!(f, ": 1700000000:0;git status").unwrap();
        let entries = load(Some(f.path()), 100).unwrap();
        // Newest first, consecutive dupes collapsed.
        assert_eq!(entries, vec!["git status", "pwd", "ls"]);
    }

    #[test]
    fn load_respects_limit() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for i in 0..10 {
            writeln!(f, "cmd{i}").unwrap();
        }
        let entries = load(Some(f.path()), 3).unwrap();
        assert_eq!(entries, vec!["cmd9", "cmd8", "cmd7"]);
    }
}
