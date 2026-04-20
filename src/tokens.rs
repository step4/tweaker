use anyhow::Result;

#[derive(Debug, Clone)]
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
            let needs_quote = t.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c));
            Token {
                text: t,
                quoted: needs_quote,
            }
        })
        .collect())
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
