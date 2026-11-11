//! SQL for search terms on a message of a thread (see `mail::search`).

use super::*;

impl Store {
    /// A message condition matching any of `values` as sender and/or
    /// recipient (To, Cc, Bcc). Full addresses match exactly; anything else
    /// matches part of a name or address.
    fn any_address(values: &[String], sender: bool, recipient: bool, p: &mut Vec<rusqlite::types::Value>) -> String {
        use rusqlite::types::Value as V;
        let mut ors = vec![];
        let like = |s: &str| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        for v in values {
            let v = v.to_lowercase();
            let exact = crate::mail::search::is_full_address(&v);
            if sender {
                if exact {
                    ors.push("lower(m.from_email)=?");
                    p.push(V::Text(v.clone()));
                } else {
                    ors.push(r"lower(m.from_email) LIKE ? ESCAPE '\' OR lower(m.from_name) LIKE ? ESCAPE '\'");
                    p.push(V::Text(like(&v)));
                    p.push(V::Text(like(&v)));
                }
            }
            if recipient {
                // Addresses are stored as JSON objects; an exact match looks for the whole email value.
                let needle = if exact { like(&format!("\"email\":{}", serde_json::Value::String(v.clone()))) } else { like(&v) };
                ors.push(r"lower(m.to_json) LIKE ? ESCAPE '\' OR lower(m.cc_json) LIKE ? ESCAPE '\' OR lower(m.bcc_json) LIKE ? ESCAPE '\'");
                for _ in 0..3 {
                    p.push(V::Text(needle.clone()));
                }
            }
        }
        format!("({})", ors.join(" OR "))
    }

    /// SQL for one search term on message `m`, appending its params.
    pub(super) fn term_sql(term: &MsgTerm, p: &mut Vec<rusqlite::types::Value>) -> String {
        use rusqlite::types::Value as V;
        match term {
            // An empty list (`from:,`) asks for nothing.
            MsgTerm::From(v) | MsgTerm::To(v) | MsgTerm::With(v) | MsgTerm::InMailbox(v) if v.is_empty() => "1".into(),
            MsgTerm::From(v) => Self::any_address(v, true, false, p),
            MsgTerm::To(v) => Self::any_address(v, false, true, p),
            MsgTerm::With(v) => Self::any_address(v, true, true, p),
            MsgTerm::Subject(s) => {
                p.push(V::Text(format!("%{}%", s.to_lowercase())));
                "lower(m.subject) LIKE ?".into()
            }
            MsgTerm::HasAttachment => "m.has_attachments=1".into(),
            MsgTerm::AttachmentName(v) | MsgTerm::AttachmentType(v) if v.is_empty() => "1".into(),
            MsgTerm::AttachmentName(v) | MsgTerm::AttachmentType(v) => {
                let ext = matches!(term, MsgTerm::AttachmentType(_));
                let esc = |s: &str| s.to_lowercase().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
                let mut ors = vec![];
                for x in v {
                    ors.push(r"lower(json_extract(j.value,'$.filename')) LIKE ? ESCAPE '\'");
                    p.push(V::Text(if ext { format!("%.{}", esc(x)) } else { format!("%{}%", esc(x)) }));
                }
                // Not images a message's HTML shows in place (logos in
                // signatures), nor Gmail's AMP bodies: the same rule as
                // `listed_attachment` in the AI folder's history.
                format!(
                    "EXISTS (SELECT 1 FROM json_each(m.attachments_json) j WHERE ({}) \
                     AND lower(json_extract(j.value,'$.mime_type'))<>'text/x-amp-html' \
                     AND NOT (json_extract(j.value,'$.inline') AND json_extract(j.value,'$.content_id') IS NOT NULL \
                              AND lower(json_extract(j.value,'$.mime_type')) LIKE 'image/%'))",
                    ors.join(" OR ")
                )
            }
            MsgTerm::Unread => "m.seen=0".into(),
            MsgTerm::Flagged => "m.flagged=1".into(),
            MsgTerm::Favorite => "EXISTS (SELECT 1 FROM favorite_senders fs WHERE fs.email=lower(m.from_email))".into(),
            MsgTerm::Before(d) => {
                p.push(V::Integer(d.timestamp_millis()));
                "m.date<?".into()
            }
            MsgTerm::After(d) => {
                p.push(V::Integer(d.timestamp_millis()));
                "m.date>=?".into()
            }
            MsgTerm::InMailbox(names) => {
                let ph = vec!["?"; names.len()].join(",");
                for _ in 0..2 {
                    p.extend(names.iter().map(|n| V::Text(n.to_lowercase())));
                }
                format!(
                    "EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=m.id AND (lower(mb.name) IN ({ph}) OR mb.role IN ({ph})))"
                )
            }
            MsgTerm::Text(q) => {
                p.push(V::Text(q.clone()));
                "m.rowid IN (SELECT rowid FROM message_fts WHERE message_fts MATCH ?)".into()
            }
        }
    }
}
