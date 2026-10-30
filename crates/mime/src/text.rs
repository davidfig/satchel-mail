//! Text helpers: quoting, snippets and subjects.

use super::*;

pub(crate) fn is_attribution_line(line: &str) -> bool {
    let l = line.trim();
    (l.starts_with("On ") && l.ends_with("wrote:"))
        || (l.starts_with("Le ") && l.ends_with("a écrit :"))
        || (l.starts_with("Am ") && l.ends_with("schrieb:"))
        || l == "-----Original Message-----"
        || l.starts_with("---------- Forwarded message")
}

/// Where quoted history starts at line `i`, beyond what one line shows: an
/// "On X wrote:" wrapped over two or three lines, an Outlook rule (a line of
/// underscores) or an Outlook header block ("From: …" then "Sent: …").
fn quote_starts_at(lines: &[&str], i: usize) -> bool {
    let l = lines[i].trim();
    if l.len() >= 20 && l.chars().all(|c| c == '_') {
        return true;
    }
    let next = || lines[i + 1..].iter().map(|l| l.trim()).filter(|l| !l.is_empty()).take(2);
    if l.starts_with("On ") && !l.ends_with('.') {
        let mut joined = l.to_string();
        for n in lines[i + 1..].iter().take(2) {
            joined.push(' ');
            joined.push_str(n.trim());
            if is_attribution_line(&joined) {
                return true;
            }
        }
    }
    let l = l.trim_start_matches('*');
    (l.starts_with("From:") || l.starts_with("De :") || l.starts_with("Von:"))
        && next().any(|n| {
            let n = n.trim_start_matches('*');
            n.starts_with("Sent:") || n.starts_with("Date:") || n.starts_with("Envoyé :") || n.starts_with("Gesendet:")
        })
}

/// Remove quoted history ("> ..." lines, "On X wrote:" attributions and
/// Outlook header blocks) and trailing signatures, leaving what this message
/// actually added.
pub fn strip_quotes(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<&str> = vec![];
    for (i, &line) in lines.iter().enumerate() {
        if is_attribution_line(line) || quote_starts_at(&lines, i) {
            break;
        }
        if line.trim_start().starts_with('>') {
            continue;
        }
        if line == "-- " {
            break;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

pub fn make_snippet(text: &str) -> String {
    let stripped = strip_quotes(text);
    let source = if stripped.trim().is_empty() { text } else { &stripped };
    let collapsed = source.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, 200)
}

pub fn truncate_chars(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Strip leading Re:/Fwd:/AW:/etc for thread grouping.
pub fn normalize_subject(s: &str) -> String {
    let mut s = s.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let mut changed = false;
        for p in ["re:", "fwd:", "fw:", "aw:", "wg:", "sv:", "vs:", "re :", "fwd :"] {
            if lower.starts_with(p) {
                s = s[p.len()..].trim_start();
                changed = true;
                break;
            }
        }
        if lower.starts_with("re[")
            && let Some(i) = s.find("]:") {
                s = s[i + 2..].trim_start();
                changed = true;
            }
        if !changed {
            break;
        }
    }
    s.to_string()
}

pub fn reply_subject(s: &str) -> String {
    let n = normalize_subject(s);
    format!("Re: {n}")
}

pub fn forward_subject(s: &str) -> String {
    let n = normalize_subject(s);
    format!("Fwd: {n}")
}

pub fn quote_for_reply(text: &str, from: Option<&Address>, date: Option<DateTime<Utc>>) -> String {
    let who = from.map(|a| a.display()).unwrap_or_else(|| "someone".into());
    let when = date
        .map(|d| d.format("%a, %b %-d, %Y at %-I:%M %p UTC").to_string())
        .unwrap_or_default();
    let mut out = format!("\n\nOn {when}, {who} wrote:\n");
    for line in text.lines() {
        if line.starts_with('>') {
            out.push('>');
        } else {
            out.push_str("> ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub fn forward_body(p: &Parsed) -> String {
    let mut out = String::from("\n\n---------- Forwarded message ---------\n");
    if let Some(f) = &p.from {
        out.push_str(&format!("From: {}\n", f.display()));
    }
    if let Some(d) = p.date {
        out.push_str(&format!("Date: {}\n", d.to_rfc2822()));
    }
    out.push_str(&format!("Subject: {}\n", p.subject));
    if !p.to.is_empty() {
        out.push_str(&format!(
            "To: {}\n",
            p.to.iter().map(|a| a.display()).collect::<Vec<_>>().join(", ")
        ));
    }
    out.push('\n');
    out.push_str(&p.text);
    out
}
