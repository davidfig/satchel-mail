//! Full-text search over message subjects, participants and bodies.
//!
//! Only the process running sync holds the index writer; agent processes
//! (e.g. `satchel mcp`) open it read-only and pick up commits automatically.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    writer: Option<Mutex<IndexWriter>>,
    f_id: Field,
    f_account: Field,
    f_subject: Field,
    f_from: Field,
    f_to: Field,
    f_body: Field,
    f_attach: Field,
}

pub struct IndexDoc<'a> {
    pub id: &'a str,
    pub account_id: &'a str,
    pub subject: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub body: &'a str,
    pub attachment_names: &'a str,
}

fn schema() -> Schema {
    let mut b = Schema::builder();
    b.add_text_field("id", STRING | STORED);
    b.add_text_field("account", STRING);
    b.add_text_field("subject", TEXT);
    b.add_text_field("from", TEXT);
    b.add_text_field("to", TEXT);
    b.add_text_field("body", TEXT);
    b.add_text_field("attach", TEXT);
    b.build()
}

impl SearchIndex {
    pub fn open(dir: &Path, writable: bool) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let index = match Index::open_in_dir(dir) {
            Ok(i) => i,
            Err(_) if writable => Index::create_in_dir(dir, schema())?,
            Err(e) => return Err(e.into()),
        };
        Self::from_index(index, writable)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_index(Index::create_in_ram(schema()), true)
    }

    fn from_index(index: Index, writable: bool) -> Result<Self> {
        let s = index.schema();
        let reader = index.reader_builder().reload_policy(ReloadPolicy::OnCommitWithDelay).try_into()?;
        let writer = if writable { Some(Mutex::new(index.writer(40_000_000)?)) } else { None };
        Ok(Self {
            f_id: s.get_field("id")?,
            f_account: s.get_field("account")?,
            f_subject: s.get_field("subject")?,
            f_from: s.get_field("from")?,
            f_to: s.get_field("to")?,
            f_body: s.get_field("body")?,
            f_attach: s.get_field("attach")?,
            index,
            reader,
            writer,
        })
    }

    pub fn writable(&self) -> bool {
        self.writer.is_some()
    }

    pub fn add(&self, docs: &[IndexDoc]) -> Result<()> {
        let Some(w) = &self.writer else { return Ok(()) };
        let w = w.lock().unwrap();
        for d in docs {
            w.delete_term(Term::from_field_text(self.f_id, d.id));
            w.add_document(doc!(
                self.f_id => d.id,
                self.f_account => d.account_id,
                self.f_subject => d.subject,
                self.f_from => d.from,
                self.f_to => d.to,
                self.f_body => d.body,
                self.f_attach => d.attachment_names,
            ))?;
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) {
        if let Some(w) = &self.writer {
            w.lock().unwrap().delete_term(Term::from_field_text(self.f_id, id));
        }
    }

    pub fn commit(&self) -> Result<()> {
        if let Some(w) = &self.writer {
            w.lock().unwrap().commit()?;
        }
        self.reader.reload()?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        if let Some(w) = &self.writer {
            let mut w = w.lock().unwrap();
            w.delete_all_documents()?;
            w.commit()?;
        }
        Ok(())
    }

    /// Free-text search; returns matching message ids, best first.
    pub fn search(&self, text: &str, limit: usize) -> Result<Vec<String>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(vec![]);
        }
        let searcher = self.reader.searcher();
        let mut qp = QueryParser::for_index(&self.index, vec![self.f_subject, self.f_from, self.f_to, self.f_body, self.f_attach]);
        qp.set_conjunction_by_default();
        qp.set_field_boost(self.f_subject, 3.0);
        qp.set_field_boost(self.f_from, 2.0);
        let (query, _errors) = qp.parse_query_lenient(text);
        let top = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut out = vec![];
        for (_score, addr) in top {
            let d: TantivyDocument = searcher.doc(addr)?;
            if let Some(id) = d.get_first(self.f_id).and_then(|v| v.as_str()) {
                out.push(id.to_string());
            }
        }
        let _ = self.f_account;
        Ok(out)
    }
}

/// A parsed search box query: operators become structured filters, the rest is free text.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ParsedQuery {
    pub text: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub has_attachment: bool,
    pub unread: bool,
    pub flagged: bool,
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
            "from" => p.from = Some(v),
            "to" => p.to = Some(v),
            "subject" => p.subject = Some(v),
            "has" if v.eq_ignore_ascii_case("attachment") => p.has_attachment = true,
            "is" if v.eq_ignore_ascii_case("unread") => p.unread = true,
            "is" if v.eq_ignore_ascii_case("flagged") || v.eq_ignore_ascii_case("starred") => p.flagged = true,
            "before" => p.before = parse_date(&v),
            "after" | "since" => p.after = parse_date(&v),
            "in" => p.in_mailbox = Some(v),
            _ => free.push(t),
        }
    }
    p.text = free.join(" ");
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_parsing() {
        let p = parse_query("from:bob budget \"q3 numbers\" is:unread has:attachment after:2026-01-02 in:inbox");
        assert_eq!(p.from.as_deref(), Some("bob"));
        assert!(p.unread && p.has_attachment);
        assert_eq!(p.in_mailbox.as_deref(), Some("inbox"));
        assert_eq!(p.text, "budget \"q3 numbers\"");
        assert!(p.after.is_some());
    }

    #[test]
    fn index_and_search() {
        let i = SearchIndex::in_memory().unwrap();
        i.add(&[
            IndexDoc { id: "a", account_id: "x", subject: "Q3 budget", from: "Carol", to: "alice", body: "numbers due friday", attachment_names: "q3-budget.pdf" },
            IndexDoc { id: "b", account_id: "x", subject: "Lunch", from: "Bob", to: "alice", body: "tacos?", attachment_names: "" },
        ])
        .unwrap();
        i.commit().unwrap();
        assert_eq!(i.search("budget", 10).unwrap(), vec!["a"]);
        assert_eq!(i.search("tacos", 10).unwrap(), vec!["b"]);
        assert!(i.search("nothing-here", 10).unwrap().is_empty());
    }
}
