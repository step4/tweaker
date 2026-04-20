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

pub fn join(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|t| {
            if t.quoted || t.text.is_empty() {
                shell_words::quote(&t.text).into_owned()
            } else {
                t.text.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generate a short selector label for each token: 1..9, then a..z.
pub fn label(i: usize) -> Option<char> {
    match i {
        0..=8 => char::from_digit((i + 1) as u32, 10),
        9..=34 => Some((b'a' + (i - 9) as u8) as char),
        _ => None,
    }
}

pub fn index_for(key: char) -> Option<usize> {
    if key.is_ascii_digit() && key != '0' {
        return Some(key as usize - '1' as usize);
    }
    if key.is_ascii_lowercase() {
        return Some(9 + (key as usize - 'a' as usize));
    }
    None
}
