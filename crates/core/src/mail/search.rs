//! Search query parsing. Operators (`from:`, `is:unread`, …) become structured
//! filters; the remaining free text becomes an SQLite FTS5 MATCH expression.

/// A parsed search box query: operators become structured filters, the rest is free text.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ParsedQuery {
    pub text: String,
    /// Each operator may list several addresses (`from:a@x,b@y`); any one matches.
    pub from: Vec<String>,
    pub to: Vec<String>,
    /// `with:` — the person sent the message or was a recipient.
    pub with: Vec<String>,
    pub subject: Option<String>,
    pub has_attachment: bool,
    pub unread: bool,
    pub flagged: bool,
    /// `is:favorite` — from a favorite sender.
    pub favorite: bool,
    pub before: Option<chrono::DateTime<chrono::Utc>>,
    pub after: Option<chrono::DateTime<chrono::Utc>>,
    pub in_mailbox: Option<String>,
}

fn parse_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{NaiveDate, TimeZone, Utc};
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(s, "%Y/%m/%d"))
        .ok()?;
    Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?))
}

/// Split respecting double quotes.
fn tokens(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut q = false;
    for c in s.chars() {
        match c {
            '"' => {
                q = !q;
                cur.push(c);
            }
            c if c.is_whitespace() && !q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn list(v: &str) -> Vec<String> {
    v.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// A complete address (`bob@x.test`) is matched exactly; anything else
/// (`bob`, `@x.test`) matches part of a name or address.
pub fn is_full_address(v: &str) -> bool {
    matches!(v.split_once('@'), Some((l, d)) if !l.is_empty() && !d.is_empty())
}

pub fn parse_query(q: &str) -> ParsedQuery {
    let mut p = ParsedQuery::default();
    let mut free = vec![];
    for t in tokens(q) {
        let (k, v) = match t.split_once(':') {
            Some((k, v)) if !v.is_empty() => (k.to_ascii_lowercase(), v.trim_matches('"').to_string()),
            _ => {
                free.push(t);
                continue;
            }
        };
        match k.as_str() {
            "from" => p.from.extend(list(&v)),
            "to" => p.to.extend(list(&v)),
            "with" => p.with.extend(list(&v)),
            "subject" => p.subject = Some(v),
            "has" if v.eq_ignore_ascii_case("attachment") => p.has_attachment = true,
            "is" if v.eq_ignore_ascii_case("unread") => p.unread = true,
            "is" if v.eq_ignore_ascii_case("flagged") || v.eq_ignore_ascii_case("starred") => p.flagged = true,
            "is" if v.eq_ignore_ascii_case("favorite") || v.eq_ignore_ascii_case("favourite") => p.favorite = true,
            "before" => p.before = parse_date(&v),
            "after" | "since" => p.after = parse_date(&v),
            "in" => p.in_mailbox = Some(v),
            _ => free.push(t),
        }
    }
    p.text = free.join(" ");
    p
}

/// Turn free text into a safe FTS5 expression: every bare word is a prefix
/// term, quoted text is a phrase, and all terms must match. Returns None when
/// nothing searchable remains.
pub fn fts_query(text: &str) -> Option<String> {
    let mut parts = vec![];
    for t in tokens(text) {
        let quoted = t.len() >= 2 && t.starts_with('"') && t.ends_with('"');
        let inner = t.trim_matches('"');
        // Keep only characters the tokenizer indexes; everything else separates words.
        let words: Vec<String> = inner
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_string())
            .collect();
        if words.is_empty() {
            continue;
        }
        if quoted {
            parts.push(format!("\"{}\"", words.join(" ")));
        } else {
            for w in words {
                parts.push(format!("\"{w}\"*"));
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_expressions() {
        assert_eq!(fts_query("budget"), Some("\"budget\"*".into()));
        assert_eq!(fts_query("q3 \"due friday\""), Some("\"q3\"* \"due friday\"".into()));
        // Operators and punctuation can't break out of the expression.
        assert_eq!(fts_query("a\" OR NOT* (b) c-d"), Some("\"a\"* \"OR\"* \"NOT\"* \"b\"* \"c\"* \"d\"*".into()));
        assert_eq!(fts_query("bob@x.test"), Some("\"bob\"* \"x\"* \"test\"*".into()));
        assert_eq!(fts_query("  ... "), None);
    }

    #[test]
    fn query_parsing() {
        let p = parse_query("from:bob budget \"q3 numbers\" is:unread has:attachment after:2026-01-02 in:inbox");
        assert_eq!(p.from, vec!["bob"]);
        assert!(p.unread && p.has_attachment);
        assert_eq!(p.in_mailbox.as_deref(), Some("inbox"));
        assert_eq!(p.text, "budget \"q3 numbers\"");
        assert!(p.after.is_some());
    }

    #[test]
    fn favorites() {
        assert!(parse_query("is:favorite").favorite);
        assert!(parse_query("IS:Favourite budget").favorite);
        assert!(!parse_query("is:flagged").favorite);
    }

    #[test]
    fn address_lists() {
        let p = parse_query("with:bob@x.test,Bob@Work.test from:carol");
        assert_eq!(p.with, vec!["bob@x.test", "Bob@Work.test"]);
        assert_eq!(p.from, vec!["carol"]);
        assert!(p.text.is_empty());
        assert!(is_full_address("bob@x.test"));
        assert!(!is_full_address("@x.test") && !is_full_address("bob@") && !is_full_address("bob"));
    }
}
