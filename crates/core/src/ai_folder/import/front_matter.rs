//! Front matter: `key: value` lines between `---` fences. Values are plain or
//! quoted strings, inline lists (`[a, b]`) or `- item` lines under a key.

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Str(String),
    List(Vec<String>),
}

#[derive(Debug, Default)]
pub struct FrontMatter(pub Vec<(String, FmValue)>);

impl FrontMatter {
    pub(crate) fn get(&self, key: &str) -> Option<&FmValue> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            FmValue::Str(s) if !s.is_empty() => Some(s),
            FmValue::List(v) if v.len() == 1 => Some(&v[0]),
            _ => None,
        }
    }

    /// A list, accepting a single plain value as a one-item list.
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(FmValue::List(v)) => v.clone(),
            Some(FmValue::Str(s)) if !s.is_empty() => vec![s.clone()],
            _ => vec![],
        }
    }

    pub(crate) fn bool(&self, key: &str) -> Result<Option<bool>> {
        match self.str(key) {
            None => Ok(None),
            Some("true" | "yes") => Ok(Some(true)),
            Some("false" | "no") => Ok(Some(false)),
            Some(v) => bail!("{key}: expected true or false, got {v:?}"),
        }
    }
}

/// Split a Markdown file into its front matter and body.
pub fn parse(text: &str) -> Result<(FrontMatter, String)> {
    let (header, body) = split(text).ok_or_else(|| anyhow!("missing front matter: start the file with a `---` line"))?;
    let mut fm = FrontMatter::default();
    for (n, line) in header.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ").or((trimmed == "-").then_some("")) {
            match fm.0.last_mut() {
                Some((_, FmValue::List(v))) => v.push(scalar(item)),
                Some((_, v @ FmValue::Str(_))) if *v == FmValue::Str(String::new()) => *v = FmValue::List(vec![scalar(item)]),
                _ => bail!("front matter line {}: list item without a key", n + 1),
            }
            continue;
        }
        let (k, v) = trimmed.split_once(':').ok_or_else(|| anyhow!("front matter line {}: expected `key: value`", n + 1))?;
        let v = strip_comment(v.trim());
        let value = if let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            FmValue::List(split_list(inner).iter().map(|s| scalar(s)).filter(|s| !s.is_empty()).collect())
        } else {
            FmValue::Str(scalar(v))
        };
        fm.0.push((k.trim().to_lowercase(), value));
    }
    Ok((fm, body.to_string()))
}

pub(crate) fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim_start_matches('\u{feff}').strip_prefix("---")?;
    let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n'))?;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..at], &rest[at + line.len()..]));
        }
        at += line.len();
    }
    None
}

/// Drop a trailing ` # comment` outside quotes.
pub(crate) fn strip_comment(v: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in v.char_indices() {
        match quote {
            Some(q) if c == q && prev != '\\' => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' && prev.is_whitespace() => return v[..i].trim_end(),
            None => {}
        }
        prev = c;
    }
    v
}

/// Split an inline list on commas outside quotes.
pub(crate) fn split_list(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                cur.push(c);
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            None if c == ',' => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

pub(crate) fn scalar(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let mut out = String::new();
        let mut esc = false;
        for c in v[1..v.len() - 1].chars() {
            match (esc, c) {
                (true, 'n') => out.push('\n'),
                (true, c) => out.push(c),
                (false, '\\') => {
                    esc = true;
                    continue;
                }
                (false, c) => out.push(c),
            }
            esc = false;
        }
        out
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("''", "'")
    } else {
        v.to_string()
    }
}

/// A double-quoted value safe to put on one front matter line.
pub(crate) fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

/// Set (Some) or remove (None) top-level keys, keeping every other line as written.
pub fn set_fields(text: &str, fields: &[(&str, Option<String>)]) -> String {
    let Some((header, body)) = split(text) else { return text.to_string() };
    let mut lines: Vec<String> = vec![];
    let mut skipping = false;
    for line in header.lines() {
        let is_item = line.trim_start().starts_with('-');
        if skipping && (is_item || line.starts_with(' ')) {
            continue;
        }
        skipping = false;
        let key = line.split_once(':').map(|(k, _)| k.trim().to_lowercase());
        if !line.starts_with(' ') && key.as_deref().is_some_and(|k| fields.iter().any(|(f, _)| *f == k)) {
            skipping = true;
            continue;
        }
        lines.push(line.to_string());
    }
    for (k, v) in fields {
        if let Some(v) = v {
            lines.push(format!("{k}: {v}"));
        }
    }
    format!("---\n{}\n---\n{body}", lines.join("\n"))
}
