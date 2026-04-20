use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Logical (unquoted) value — what the shell sees.
    pub text: String,
    /// Exact form as it appeared in the original command (e.g. `'^g'`).
    /// Used verbatim on render so quoting style is never lost.
    /// After an edit, recomputed via `quote_for_render`.
    pub original: String,
}

/// Split `cmd` into tokens, preserving each token's original quoting.
/// Unlike `shell_words::split`, this keeps `text` (logical) and `original`
/// (source form including quotes) so that `render` can reconstruct exactly.
pub fn split(cmd: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = cmd.chars().peekable();

    loop {
        // Skip inter-token whitespace.
        while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        let mut text = String::new();
        let mut original = String::new();

        // Consume one token (may be multiple adjacent quoted/unquoted segments).
        loop {
            let Some(&ch) = chars.peek() else { break };
            if ch.is_whitespace() {
                break;
            }
            chars.next();
            match ch {
                '\'' => {
                    original.push('\'');
                    loop {
                        match chars.next() {
                            None => anyhow::bail!("unterminated single quote in: {cmd}"),
                            Some('\'') => {
                                original.push('\'');
                                break;
                            }
                            Some(c) => {
                                text.push(c);
                                original.push(c);
                            }
                        }
                    }
                }
                '"' => {
                    original.push('"');
                    loop {
                        match chars.next() {
                            None => anyhow::bail!("unterminated double quote in: {cmd}"),
                            Some('"') => {
                                original.push('"');
                                break;
                            }
                            Some('\\') => {
                                original.push('\\');
                                match chars.next() {
                                    None => anyhow::bail!("trailing backslash in: {cmd}"),
                                    Some(c) => {
                                        original.push(c);
                                        // Inside double quotes, \ only escapes these chars.
                                        if matches!(c, '"' | '\\' | '$' | '`' | '\n') {
                                            text.push(c);
                                        } else {
                                            text.push('\\');
                                            text.push(c);
                                        }
                                    }
                                }
                            }
                            Some(c) => {
                                text.push(c);
                                original.push(c);
                            }
                        }
                    }
                }
                '\\' => {
                    original.push('\\');
                    match chars.next() {
                        None => {} // trailing backslash — ignore
                        Some('\n') => {} // line continuation
                        Some(c) => {
                            original.push(c);
                            text.push(c);
                        }
                    }
                }
                c => {
                    text.push(c);
                    original.push(c);
                }
            }
        }

        tokens.push(Token { text, original });
    }
    Ok(tokens)
}

pub fn render(tokens: &[Token]) -> String {
    render_with_spans(tokens).0
}

/// Render joined command and return the char-column span (start, len) per token.
/// Uses each token's `original` field verbatim, so quoting is preserved exactly.
pub fn render_with_spans(tokens: &[Token]) -> (String, Vec<(usize, usize)>) {
    let mut out = String::new();
    let mut spans = Vec::with_capacity(tokens.len());
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let start = out.chars().count();
        let len = t.original.chars().count();
        out.push_str(&t.original);
        spans.push((start, len));
    }
    (out, spans)
}

/// Quote `text` for re-insertion as a new token (e.g. after an edit).
/// Uses shell_words::quote, which is correct for POSIX shells.
pub fn quote_for_render(text: &str) -> String {
    shell_words::quote(text).into_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStyle {
    None,
    Single,
    Double,
}

impl QuoteStyle {
    /// Detect the quoting style from a token's original form.
    /// Mixed forms (e.g. `foo'bar'`) are treated as None.
    pub fn from_original(original: &str) -> Self {
        if original.starts_with('\'') {
            Self::Single
        } else if original.starts_with('"') {
            Self::Double
        } else {
            Self::None
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::None => Self::Single,
            Self::Single => Self::Double,
            Self::Double => Self::None,
        }
    }

    /// Produce the shell form for `text` under this quoting style.
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::None => shell_words::quote(text).into_owned(),
            Self::Single => {
                // ' can't appear inside '...'; escape via closing-quote trick.
                let escaped = text.replace('\'', "'\\''");
                format!("'{escaped}'")
            }
            Self::Double => {
                let escaped = text
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('$', "\\$")
                    .replace('`', "\\`");
                format!("\"{escaped}\"")
            }
        }
    }

    /// Number of leading quote characters (for cursor offset in edit display).
    pub fn prefix_len(self) -> usize {
        match self {
            Self::None => 0,
            Self::Single | Self::Double => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "no quote",
            Self::Single => "single '",
            Self::Double => "double \"",
        }
    }
}

/// Selector label for each token: 1..9, then A..Z. Uppercase is deliberate —
/// lowercase letters are reserved as command prefixes (d, a, …).
pub fn label(i: usize) -> Option<char> {
    match i {
        0..=8 => char::from_digit((i + 1) as u32, 10),
        9..=34 => Some((b'A' + (i - 9) as u8) as char),
        _ => None,
    }
}

pub fn index_for(key: char) -> Option<usize> {
    if key.is_ascii_digit() && key != '0' {
        return Some(key as usize - '1' as usize);
    }
    if key.is_ascii_uppercase() {
        return Some(9 + (key as usize - 'A' as usize));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(tokens: &[Token]) -> Vec<&str> {
        tokens.iter().map(|t| t.text.as_str()).collect()
    }

    #[test]
    fn split_plain() {
        let t = split("git commit -m hello").unwrap();
        assert_eq!(texts(&t), ["git", "commit", "-m", "hello"]);
        // No quoting — original equals text.
        assert!(t.iter().all(|tok| tok.original == tok.text));
    }

    #[test]
    fn split_preserves_double_quoted_original() {
        let t = split(r#"git commit -m "hi there""#).unwrap();
        assert_eq!(texts(&t), ["git", "commit", "-m", "hi there"]);
        assert_eq!(t[3].original, r#""hi there""#, "original must include the quotes");
    }

    #[test]
    fn split_preserves_single_quoted_original() {
        let t = split("bindkey '^g' tweaker-widget").unwrap();
        assert_eq!(t[1].text, "^g");
        assert_eq!(t[1].original, "'^g'", "original must keep the single quotes");
    }

    #[test]
    fn render_roundtrips_plain() {
        let cmd = "ls -la /tmp";
        assert_eq!(render(&split(cmd).unwrap()), cmd);
    }

    #[test]
    fn render_preserves_original_quoting() {
        // The whole command must round-trip character-for-character.
        for cmd in [
            "bindkey '^g' tweaker-widget",
            r#"git commit -m "fix: typo""#,
            "echo '*.rs'",
            r"echo \$HOME",
        ] {
            assert_eq!(render(&split(cmd).unwrap()), cmd, "render did not roundtrip: {cmd}");
        }
    }

    #[test]
    fn spans_line_up_with_rendered_string() {
        let cmd = "git commit -m 'hi there'";
        let tokens = split(cmd).unwrap();
        let (rendered, spans) = render_with_spans(&tokens);
        for (i, (start, len)) in spans.iter().enumerate() {
            let slice: String = rendered.chars().skip(*start).take(*len).collect();
            // Each span in the rendered string re-parses to the original token text.
            assert_eq!(split(&slice).unwrap()[0].text, tokens[i].text, "span {i}");
        }
    }

    #[test]
    fn edited_token_gets_requoted_for_render() {
        // After an edit, original is recomputed. A token with whitespace must be quoted.
        let text = "hi there".to_string();
        let original = quote_for_render(&text);
        let reparsed = split(&original).unwrap();
        assert_eq!(reparsed[0].text, "hi there");
    }

    #[test]
    fn label_index_roundtrip() {
        for i in 0..35 {
            let lbl = label(i).expect("label in range");
            assert_eq!(index_for(lbl), Some(i), "roundtrip for index {i}");
        }
        assert_eq!(label(35), None);
    }

    #[test]
    fn digits_and_uppercase_only() {
        assert_eq!(index_for('0'), None);
        assert_eq!(index_for('a'), None, "lowercase is for commands");
        assert_eq!(index_for('d'), None);
        assert_eq!(index_for('1'), Some(0));
        assert_eq!(index_for('A'), Some(9));
    }
}
