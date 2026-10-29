//! Search query parsing. Operators (`from:`, `is:unread`, …) become structured
//! conditions; the remaining free text becomes an SQLite FTS5 MATCH expression.
//! Every term must hold (AND); a comma list inside one operator matches any of
//! its values (OR); a leading `-` negates a term. Smart folders store queries
//! in this language, so relative dates (`newer_than:7d`) keep them current.

use chrono::{DateTime, Duration, Utc};

use crate::store::{MsgCond, MsgTerm};

/// A parsed search box query.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ParsedQuery {
    /// Conditions on a message of the thread; all must hold for the same message.
    pub conds: Vec<MsgCond>,
    /// `account:` values (part of an account's address) and whether negated.
    pub accounts: Vec<(Vec<String>, bool)>,
    /// `in:anywhere`: include Trash and Junk.
    pub anywhere: bool,
}

impl ParsedQuery {
    /// Whether the query says where to look (`in:`), which replaces the
    /// default of leaving out Trash and Junk.
    pub fn names_mailbox(&self) -> bool {
        self.anywhere || self.conds.iter().any(|c| !c.negate && matches!(c.term, MsgTerm::InMailbox(_)))
    }
}

fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    use chrono::{NaiveDate, TimeZone};
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(s, "%Y/%m/%d"))
        .ok()?;
    Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?))
}

/// `7d`, `2w`, `3m`, `1y` (days, weeks, months of 30 days, years of 365).
fn parse_age(s: &str) -> Option<Duration> {
    let unit = s.chars().last()?;
    let n: i64 = s[..s.len() - unit.len_utf8()].parse().ok().filter(|n| *n >= 0)?;
    let days = match unit.to_ascii_lowercase() {
        'd' => 1,
        'w' => 7,
        'm' => 30,
        'y' => 365,
        _ => return None,
    };
    Some(Duration::days(n.checked_mul(days)?.min(365 * 1000)))
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

/// The term an `op:value` token stands for, if it is an operator we know.
fn operator(k: &str, v: &str, now: DateTime<Utc>) -> Option<MsgTerm> {
    let is = |x: &str| v.eq_ignore_ascii_case(x);
    Some(match k {
        "from" => MsgTerm::From(list(v)),
        "to" => MsgTerm::To(list(v)),
        "with" => MsgTerm::With(list(v)),
        "subject" => MsgTerm::Subject(v.to_string()),
        "has" if is("attachment") => MsgTerm::HasAttachment,
        "is" if is("unread") => MsgTerm::Unread,
        "is" if is("flagged") || is("starred") => MsgTerm::Flagged,
        "before" => MsgTerm::Before(parse_date(v)?),
        "after" | "since" => MsgTerm::After(parse_date(v)?),
        "older_than" => MsgTerm::Before(now - parse_age(v)?),
        "newer_than" => MsgTerm::After(now - parse_age(v)?),
        "in" => MsgTerm::InMailbox(list(v)),
        _ => return None,
    })
}

pub fn parse_query(q: &str) -> ParsedQuery {
    parse_query_at(q, Utc::now())
}

/// [`parse_query`] with relative dates measured from `now`.
pub fn parse_query_at(q: &str, now: DateTime<Utc>) -> ParsedQuery {
    let mut p = ParsedQuery::default();
    let mut free = vec![];
    let mut not_free = vec![];
    for t in tokens(q) {
        let (negate, body) = match t.strip_prefix('-') {
            Some(rest) if !rest.is_empty() => (true, rest.to_string()),
            _ => (false, t.clone()),
        };
        let Some((k, v)) = body.split_once(':').filter(|(_, v)| !v.is_empty()) else {
            if negate { not_free.push(body) } else { free.push(body) }
            continue;
        };
        let k = k.to_ascii_lowercase();
        let v = v.trim_matches('"');
        // `is:read` and `is:unflagged` are the negations of `is:unread` and `is:flagged`.
        let (v, negate) = match (k.as_str(), v.to_ascii_lowercase().as_str()) {
            ("is", "read") => ("unread", !negate),
            ("is", "unflagged" | "unstarred") => ("flagged", !negate),
            _ => (v, negate),
        };
        if k == "in" && v.eq_ignore_ascii_case("anywhere") {
            p.anywhere = !negate;
        } else if k == "account" {
            p.accounts.push((list(v), negate));
        } else if let Some(term) = operator(&k, v, now) {
            p.conds.push(MsgCond { term, negate });
        } else if negate {
            not_free.push(body);
        } else {
            free.push(body);
        }
    }
    if let Some(q) = fts_query(&free.join(" ")) {
        p.conds.push(MsgCond { term: MsgTerm::Text(q), negate: false });
    }
    for w in not_free {
        if let Some(q) = fts_query(&w) {
            p.conds.push(MsgCond { term: MsgTerm::Text(q), negate: true });
        }
    }
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

    fn has(p: &ParsedQuery, negate: bool, t: MsgTerm) -> bool {
        p.conds.iter().any(|c| c.negate == negate && c.term == t)
    }

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
        assert!(has(&p, false, MsgTerm::From(vec!["bob".into()])));
        assert!(has(&p, false, MsgTerm::Unread) && has(&p, false, MsgTerm::HasAttachment));
        assert!(has(&p, false, MsgTerm::After(parse_date("2026-01-02").unwrap())));
        assert!(has(&p, false, MsgTerm::InMailbox(vec!["inbox".into()])));
        assert!(has(&p, false, MsgTerm::Text("\"budget\"* \"q3 numbers\"".into())));
        assert!(p.names_mailbox());
    }

    #[test]
    fn address_lists() {
        let p = parse_query("with:bob@x.test,Bob@Work.test from:carol");
        assert_eq!(p.conds[0].term, MsgTerm::With(vec!["bob@x.test".into(), "Bob@Work.test".into()]));
        assert_eq!(p.conds[1].term, MsgTerm::From(vec!["carol".into()]));
        assert_eq!(p.conds.len(), 2);
        assert!(is_full_address("bob@x.test"));
        assert!(!is_full_address("@x.test") && !is_full_address("bob@") && !is_full_address("bob"));
    }

    #[test]
    fn negation() {
        let p = parse_query("-from:noreply is:read is:unflagged -in:inbox report -draft -\"old news\"");
        assert!(has(&p, true, MsgTerm::From(vec!["noreply".into()])));
        assert!(has(&p, true, MsgTerm::Unread) && has(&p, true, MsgTerm::Flagged));
        assert!(has(&p, true, MsgTerm::InMailbox(vec!["inbox".into()])));
        assert!(has(&p, true, MsgTerm::Text("\"draft\"*".into())));
        assert!(has(&p, true, MsgTerm::Text("\"old news\"".into())));
        assert!(has(&p, false, MsgTerm::Text("\"report\"*".into())));
        // A negated mailbox doesn't replace the Trash and Junk default.
        assert!(!p.names_mailbox());
        // A lone dash is text, not a negation.
        assert!(parse_query("-").conds.is_empty());
    }

    #[test]
    fn relative_dates() {
        let now = parse_date("2026-10-29").unwrap();
        let p = parse_query_at("newer_than:7d older_than:1y", now);
        assert_eq!(p.conds[0].term, MsgTerm::After(parse_date("2026-10-22").unwrap()));
        assert_eq!(p.conds[1].term, MsgTerm::Before(now - Duration::days(365)));
        assert_eq!(parse_query_at("newer_than:2W", now).conds[0].term, MsgTerm::After(now - Duration::days(14)));
        // Nonsense stays free text instead of becoming a filter.
        assert!(matches!(parse_query_at("newer_than:soon", now).conds[0].term, MsgTerm::Text(_)));
        assert!(matches!(parse_query_at("newer_than:é", now).conds[0].term, MsgTerm::Text(_)));
    }

    #[test]
    fn accounts_and_anywhere() {
        let p = parse_query("account:work -account:old@x.test,spare in:anywhere");
        assert_eq!(p.accounts, vec![(vec!["work".to_string()], false), (vec!["old@x.test".to_string(), "spare".to_string()], true)]);
        assert!(p.anywhere && p.names_mailbox());
        assert!(p.conds.is_empty());
    }
}
