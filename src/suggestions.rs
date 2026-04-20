use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestionKind {
    /// Full command replacement loaded from a tldr page.
    Example,
    /// Single flag/option extracted from a man page — appended as a new token.
    Flag,
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub description: String,
    pub example: String,
    pub kind: SuggestionKind,
}

/// Load suggestions for `cmd`. Tries the tealdeer cache first, falls back to
/// `man`, returns an empty vec if neither source yields results. Never errors.
pub fn load(cmd: &str) -> Vec<Suggestion> {
    load_tldr(cmd).unwrap_or_else(|| load_man(cmd))
}

// ─── tldr ─────────────────────────────────────────────────────────────────────

fn load_tldr(cmd: &str) -> Option<Vec<Suggestion>> {
    for dir in tldr_cache_dirs() {
        for platform in platform_subdirs() {
            // Tealdeer ≥ 0.6 uses a "tldr-pages/" subdirectory; older versions don't.
            for base in [
                dir.join("tldr-pages").join("pages").join(platform),
                dir.join("pages").join(platform),
            ] {
                let path = base.join(format!("{cmd}.md"));
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let s = parse_tldr(&content);
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
        }
    }
    None
}

fn tldr_cache_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // Respect $XDG_CACHE_HOME; fall back to the platform cache directory.
    let cache = std::env::var("XDG_CACHE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs::cache_dir);
    if let Some(c) = cache {
        dirs.push(c.join("tealdeer"));
    }
    // Tealdeer sometimes stores pages under the data directory instead.
    if let Some(d) = dirs::data_dir() {
        dirs.push(d.join("tealdeer"));
    }
    dirs
}

fn platform_subdirs() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["common", "osx"]
    } else if cfg!(target_os = "linux") {
        &["common", "linux"]
    } else {
        &["common"]
    }
}

fn parse_tldr(content: &str) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let mut desc: Option<String> = None;
    for line in content.lines() {
        let t = line.trim();
        if let Some(d) = t.strip_prefix("- ") {
            desc = Some(d.trim_end_matches(':').to_string());
        } else if t.len() > 2 && t.starts_with('`') && t.ends_with('`') {
            out.push(Suggestion {
                description: desc.take().unwrap_or_default(),
                example: t[1..t.len() - 1].to_string(),
                kind: SuggestionKind::Example,
            });
        }
    }
    out
}

// ─── man fallback ─────────────────────────────────────────────────────────────

fn load_man(cmd: &str) -> Vec<Suggestion> {
    use std::process::{Command, Stdio};

    let Ok(out) = Command::new("man")
        .arg(cmd)
        .env("MANWIDTH", "200")
        .env("MANPAGER", "cat")
        .env("MAN_KEEP_FORMATTING", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return vec![];
    };

    if !out.status.success() {
        return vec![];
    }

    let raw = String::from_utf8_lossy(&out.stdout);
    parse_man_options(&strip_backspace(&raw))
}

/// Remove overstrike formatting sequences (`char \x08 char` bold, `_ \x08 char` underline).
fn strip_backspace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\x08' {
            let trim = out.chars().next_back().map(|c| c.len_utf8()).unwrap_or(0);
            out.truncate(out.len().saturating_sub(trim));
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_man_options(text: &str) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let mut in_section = false;
    let mut pending_flag: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        // Top-level section headers have no leading whitespace.
        if !line.starts_with(' ') && !line.starts_with('\t') {
            if !trimmed.is_empty() {
                let upper = trimmed.to_ascii_uppercase();
                if upper.contains("OPTION") || upper.contains("FLAG") {
                    in_section = true;
                    pending_flag = None;
                } else if in_section {
                    break; // left the options section
                }
            }
            continue;
        }

        if !in_section || trimmed.is_empty() {
            continue;
        }

        let indent = line.len() - trimmed.len();

        // Flag lines: 2–12 spaces of indent, start with `-` followed by `-` or a letter.
        let looks_like_flag = (2..=12).contains(&indent)
            && trimmed.starts_with('-')
            && trimmed
                .chars()
                .nth(1)
                .map(|c| c == '-' || c.is_ascii_alphabetic())
                .unwrap_or(false);

        if looks_like_flag {
            let (flag, desc) = split_flag_line(trimmed);
            if !flag.is_empty() {
                if desc.is_empty() {
                    pending_flag = Some(flag);
                } else {
                    out.push(Suggestion {
                        description: desc,
                        example: flag,
                        kind: SuggestionKind::Flag,
                    });
                }
            }
        } else if let Some(flag) = pending_flag.take() {
            // Description is on the line following the flag.
            out.push(Suggestion {
                description: trimmed.to_string(),
                example: flag,
                kind: SuggestionKind::Flag,
            });
        }
    }
    out
}

/// Split a flag line like `-a, --all  do not ignore entries` into
/// `("-a", "do not ignore entries")`. Skips alternate flag forms.
fn split_flag_line(line: &str) -> (String, String) {
    let mut words = line.split_whitespace();
    let flag = words
        .next()
        .unwrap_or("")
        .trim_end_matches(',')
        .to_string();
    // Collect remaining words, skipping leading alternate-form flags (--long).
    let rest: Vec<&str> = words
        .skip_while(|w| w.starts_with('-') || w.trim_end_matches(',').starts_with('-'))
        .collect();
    (flag, rest.join(" "))
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TLDR_GIT_COMMIT: &str = r#"# git commit

> Create a commit with a message.

- Commit staged files with a message:
`git commit -m "{{message}}"`

- Commit staged files with a long message:
`git commit -m "{{title}}" -m "{{body}}"`

- Amend the last commit:
`git commit --amend`
"#;

    #[test]
    fn parse_tldr_extracts_examples() {
        let s = parse_tldr(TLDR_GIT_COMMIT);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].example, r#"git commit -m "{{message}}""#);
        assert_eq!(s[0].description, "Commit staged files with a message");
        assert!(s.iter().all(|s| s.kind == SuggestionKind::Example));
    }

    #[test]
    fn parse_tldr_skips_summary_lines() {
        let s = parse_tldr(TLDR_GIT_COMMIT);
        // Lines starting with `>` are summaries, not examples.
        assert!(s.iter().all(|s| !s.example.starts_with('>')));
    }

    #[test]
    fn load_tldr_returns_none_for_missing_page() {
        // A command name that won't have a tldr page.
        assert!(load_tldr("__no_such_cmd_xyz__").is_none());
    }

    #[test]
    fn load_tldr_reads_from_tmpdir() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let page_dir = dir.path().join("tldr-pages").join("pages").join("common");
        std::fs::create_dir_all(&page_dir).unwrap();
        let mut f = std::fs::File::create(page_dir.join("mytest.md")).unwrap();
        write!(f, "{TLDR_GIT_COMMIT}").unwrap();

        // Place the page where the loader expects it.
        let cache_page_dir = dir.path().join("cache").join("tealdeer").join("tldr-pages").join("pages").join("common");
        std::fs::create_dir_all(&cache_page_dir).unwrap();
        std::fs::copy(page_dir.join("mytest.md"), cache_page_dir.join("mytest.md")).unwrap();

        // SAFETY: test-only env mutation; tests run in a single process.
        unsafe { std::env::set_var("XDG_CACHE_HOME", dir.path().join("cache")); }
        let s = load_tldr("mytest");
        unsafe { std::env::remove_var("XDG_CACHE_HOME"); }
        assert!(s.is_some());
        assert_eq!(s.unwrap().len(), 3);
    }

    #[test]
    fn strip_backspace_removes_overstrike() {
        // Bold in man: 'a\x08a' → 'a'
        let bold = "a\x08ab\x08b";
        assert_eq!(strip_backspace(bold), "ab");
        // Underline: '_\x08a' → 'a'
        let ul = "_\x08a_\x08b";
        assert_eq!(strip_backspace(ul), "ab");
    }

    const MAN_OPTIONS: &str = "
NAME
       ls - list directory contents

OPTIONS
       -a, --all
              do not ignore entries starting with .

       -A     do not list implied . and ..

       -l     use a long listing format

OTHER
       not an option
";

    #[test]
    fn parse_man_options_extracts_flags() {
        let s = parse_man_options(MAN_OPTIONS);
        assert!(!s.is_empty());
        assert!(s.iter().all(|s| s.kind == SuggestionKind::Flag));
        let flags: Vec<&str> = s.iter().map(|s| s.example.as_str()).collect();
        assert!(flags.contains(&"-a"), "expected -a in {flags:?}");
        assert!(flags.contains(&"-A"), "expected -A in {flags:?}");
        assert!(flags.contains(&"-l"), "expected -l in {flags:?}");
    }

    #[test]
    fn parse_man_options_stops_at_next_section() {
        let s = parse_man_options(MAN_OPTIONS);
        assert!(s.iter().all(|s| s.example != "not"));
    }

    #[test]
    fn split_flag_line_handles_alternate_form() {
        let (flag, desc) = split_flag_line("-a, --all  do not ignore entries");
        assert_eq!(flag, "-a");
        assert_eq!(desc, "do not ignore entries");
    }

    #[test]
    fn split_flag_line_no_desc() {
        let (flag, desc) = split_flag_line("-v");
        assert_eq!(flag, "-v");
        assert!(desc.is_empty());
    }
}
