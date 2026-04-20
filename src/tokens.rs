use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    /// True if the token was produced from a quoted segment — preserve quoting on render.
    pub quoted: bool,
}

pub fn split(cmd: &str) -> Result<Vec<Token>> {
    // shell-words strips quoting. We still want to roundtrip safely, so re-quote on render.
    let parts = shell_words::split(cmd)?;
    Ok(parts
        .into_iter()
        .map(|t| {
            let q = needs_quote(&t);
            Token {
                text: t,
                quoted: q,
            }
        })
        .collect())
}

pub fn needs_quote(s: &str) -> bool {
    s.is_empty() || s.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c))
}

pub fn render(tokens: &[Token]) -> String {
    render_with_spans(tokens).0
}

/// Render joined command, and return the char-column span (start, len) of each token.
pub fn render_with_spans(tokens: &[Token]) -> (String, Vec<(usize, usize)>) {
    let mut out = String::new();
    let mut spans = Vec::with_capacity(tokens.len());
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let rendered = if t.quoted || t.text.is_empty() {
            shell_words::quote(&t.text).into_owned()
        } else {
            t.text.clone()
        };
        let start = out.chars().count();
        let len = rendered.chars().count();
        out.push_str(&rendered);
        spans.push((start, len));
    }
    (out, spans)
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
        assert!(t.iter().all(|tok| !tok.quoted));
    }

    #[test]
    fn split_strips_quotes_but_marks_quoted() {
        let t = split(r#"git commit -m "hi there""#).unwrap();
        assert_eq!(texts(&t), ["git", "commit", "-m", "hi there"]);
        assert!(t[3].quoted, "token with whitespace must be flagged quoted");
    }

    #[test]
    fn render_roundtrips_plain() {
        let cmd = "ls -la /tmp";
        assert_eq!(render(&split(cmd).unwrap()), cmd);
    }

    #[test]
    fn render_requotes_tokens_with_whitespace() {
        let cmd = r#"echo "hi there""#;
        let rendered = render(&split(cmd).unwrap());
        // Shell-words may pick single-quote form; accept either as long as it re-parses identically.
        assert_eq!(split(&rendered).unwrap()[1].text, "hi there");
    }

    #[test]
    fn spans_line_up_with_rendered_string() {
        let cmd = "git commit -m 'hi there'";
        let tokens = split(cmd).unwrap();
        let (rendered, spans) = render_with_spans(&tokens);
        for (i, (start, len)) in spans.iter().enumerate() {
            let slice: String = rendered.chars().skip(*start).take(*len).collect();
            // The slice at each span, once reparsed on its own, yields the original token text.
            assert_eq!(split(&slice).unwrap()[0].text, tokens[i].text, "span {i}");
        }
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
