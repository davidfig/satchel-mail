//! JMAP objects to Satchel's: addresses, flags, roles, message records.

use super::*;

/// Servers sometimes advertise their public hostname; on loopback test servers
/// rebase those URLs onto the session URL's origin.
pub(super) fn rebase(url: &str, session_url: &str) -> String {
    let (Ok(u), Ok(s)) = (url::Url::parse(url), url::Url::parse(session_url)) else { return url.to_string() };
    let loopback = |h: Option<&str>| matches!(h, Some("localhost") | Some("127.0.0.1") | Some("::1"));
    if loopback(s.host_str()) && loopback(u.host_str()) && u.origin() != s.origin() {
        let path = &url[u.origin().ascii_serialization().len()..];
        return format!("{}{}", s.origin().ascii_serialization(), path);
    }
    url.to_string()
}

pub(super) fn addresses(v: &Value) -> Vec<Address> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| Some(Address { name: x["name"].as_str().filter(|s| !s.is_empty()).map(String::from), email: x["email"].as_str()?.to_string() }))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn first_str(v: &Value) -> Option<String> {
    v.as_array().and_then(|a| a.first()).and_then(Value::as_str).map(String::from)
}

pub(super) fn flags_of(v: &Value) -> Flags {
    let k = &v["keywords"];
    Flags { seen: k["$seen"] == true, flagged: k["$flagged"] == true, answered: k["$answered"] == true, draft: k["$draft"] == true }
}

pub(super) fn role_of(v: &Value) -> MailboxRole {
    match v["role"].as_str().unwrap_or_default() {
        "inbox" => MailboxRole::Inbox,
        "sent" => MailboxRole::Sent,
        "drafts" => MailboxRole::Drafts,
        "trash" => MailboxRole::Trash,
        "junk" => MailboxRole::Junk,
        "archive" => MailboxRole::Archive,
        "all" => MailboxRole::All,
        "flagged" => MailboxRole::Flagged,
        "important" => MailboxRole::Important,
        _ => MailboxRole::Custom,
    }
}

pub(super) fn record(e: &Value) -> MessageRecord {
    let date = e["receivedAt"].as_str().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc)).unwrap_or_else(Utc::now);
    MessageRecord {
        remote_id: e["id"].as_str().unwrap_or_default().to_string(),
        remote_thread_id: e["threadId"].as_str().map(String::from),
        message_id_hdr: first_str(&e["messageId"]),
        in_reply_to: first_str(&e["inReplyTo"]),
        references: e["references"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default(),
        subject: e["subject"].as_str().unwrap_or_default().to_string(),
        from: addresses(&e["from"]).into_iter().next(),
        to: addresses(&e["to"]),
        cc: addresses(&e["cc"]),
        bcc: addresses(&e["bcc"]),
        reply_to: addresses(&e["replyTo"]),
        date,
        snippet: e["preview"].as_str().unwrap_or_default().to_string(),
        flags: flags_of(e),
        attachments: vec![],
        blob_hash: None,
        size: e["size"].as_u64().unwrap_or(0),
        ..Default::default()
    }
}

pub(super) fn mailboxes_of(e: &Value) -> Vec<(String, Option<u32>)> {
    e["mailboxIds"].as_object().map(|m| m.iter().filter(|(_, v)| **v == true).map(|(k, _)| (k.clone(), None)).collect()).unwrap_or_default()
}
