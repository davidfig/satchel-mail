//! Gmail API mail provider: labels, history-based deltas.

mod apply;
mod sync;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use chrono::{TimeZone, Utc};
use futures::StreamExt;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CredentialHandle, MailProvider, RemoteMailbox, ResolvedOp, SyncCtx};
use satchel_core::store::{MessageRecord, MessageRef};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::oauth::ApiClient;

pub use super::api_client;

const GMAIL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const META_HEADERS: [&str; 10] = ["From", "To", "Cc", "Bcc", "Reply-To", "Subject", "Date", "Message-ID", "In-Reply-To", "References"];
const CONCURRENCY: usize = 8;
fn system_role(id: &str) -> Option<MailboxRole> {
    Some(match id {
        "INBOX" => MailboxRole::Inbox,
        "SENT" => MailboxRole::Sent,
        "DRAFT" => MailboxRole::Drafts,
        "TRASH" => MailboxRole::Trash,
        "SPAM" => MailboxRole::Junk,
        "STARRED" => MailboxRole::Flagged,
        "IMPORTANT" => MailboxRole::Important,
        _ => return None,
    })
}

fn b64url_decode(s: &str) -> Result<Vec<u8>> {
    let s = s.trim_end_matches('=');
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s)?)
}

fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn flags_from_labels(labels: &[String]) -> Flags {
    Flags {
        seen: !labels.iter().any(|l| l == "UNREAD"),
        flagged: labels.iter().any(|l| l == "STARRED"),
        answered: false,
        draft: labels.iter().any(|l| l == "DRAFT"),
    }
}

fn label_ids(v: &Value) -> Vec<String> {
    v["labelIds"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

/// Build a record from a `format=metadata` message.
fn record_from_metadata(m: &Value) -> MessageRecord {
    let mut hdr: HashMap<String, String> = HashMap::new();
    for h in m["payload"]["headers"].as_array().cloned().unwrap_or_default() {
        if let (Some(n), Some(v)) = (h["name"].as_str(), h["value"].as_str()) {
            hdr.entry(n.to_ascii_lowercase()).or_insert_with(|| v.to_string());
        }
    }
    // Reuse the MIME parser for address and id parsing by feeding it a header block.
    let mut block = String::new();
    for k in META_HEADERS {
        if let Some(v) = hdr.get(&k.to_ascii_lowercase()) {
            block.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    block.push_str("\r\n");
    let p = satchel_mime::parse(block.as_bytes()).unwrap_or_default();
    let internal = m["internalDate"].as_str().and_then(|s| s.parse::<i64>().ok()).and_then(|ms| Utc.timestamp_millis_opt(ms).single());
    let labels = label_ids(m);
    MessageRecord {
        remote_id: m["id"].as_str().unwrap_or_default().to_string(),
        remote_thread_id: m["threadId"].as_str().map(String::from),
        message_id_hdr: p.message_id,
        in_reply_to: p.in_reply_to,
        references: p.references,
        subject: p.subject,
        from: p.from,
        to: p.to,
        cc: p.cc,
        bcc: p.bcc,
        reply_to: p.reply_to,
        date: internal.or(p.date).unwrap_or_else(Utc::now),
        snippet: html_unescape(m["snippet"].as_str().unwrap_or_default()),
        flags: flags_from_labels(&labels),
        attachments: vec![],
        blob_hash: None,
        size: m["sizeEstimate"].as_u64().unwrap_or(0),
    }
}

/// Mailbox memberships for a set of Gmail labels. Everything outside Trash and
/// Spam is also in the virtual All Mail mailbox.
fn memberships(labels: &[String], known: &HashSet<String>) -> Vec<(String, Option<u32>)> {
    let mut out: Vec<(String, Option<u32>)> = labels.iter().filter(|l| known.contains(*l)).map(|l| (l.clone(), None)).collect();
    if !labels.iter().any(|l| l == "TRASH" || l == "SPAM") {
        out.push(("ALL".into(), None));
    }
    out
}

fn html_unescape(s: &str) -> String {
    s.replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

pub struct GmailProvider {
    api: ApiClient,
}

impl GmailProvider {
    pub fn new(creds: CredentialHandle) -> Result<Self> {
        Ok(Self { api: api_client(creds)? })
    }

    async fn get(&self, url: &str, what: &str) -> Result<Value> {
        self.api.json(|h, t| h.get(url).bearer_auth(t), what).await
    }

    async fn post(&self, url: &str, body: Value, what: &str) -> Result<Value> {
        self.api.json(|h, t| h.post(url).bearer_auth(t).json(&body), what).await
    }

    async fn labels(&self) -> Result<Vec<RemoteMailbox>> {
        let v = self.get(&format!("{GMAIL}/labels"), "list labels").await?;
        let mut out = vec![];
        for l in v["labels"].as_array().cloned().unwrap_or_default() {
            let id = l["id"].as_str().unwrap_or_default().to_string();
            let name = l["name"].as_str().unwrap_or(&id).to_string();
            if id.starts_with("CATEGORY_") || id == "UNREAD" || id == "CHAT" || l["labelListVisibility"] == "labelHide" && l["type"] != "system" {
                continue;
            }
            let (role, display) = match system_role(&id) {
                Some(r) => (r, match r {
                    MailboxRole::Inbox => "Inbox".to_string(),
                    MailboxRole::Sent => "Sent".into(),
                    MailboxRole::Drafts => "Drafts".into(),
                    MailboxRole::Trash => "Trash".into(),
                    MailboxRole::Junk => "Spam".into(),
                    MailboxRole::Flagged => "Starred".into(),
                    MailboxRole::Important => "Important".into(),
                    _ => name.clone(),
                }),
                None if l["type"] == "system" => continue,
                None => (MailboxRole::Custom, name.rsplit('/').next().unwrap_or(&name).to_string()),
            };
            out.push(RemoteMailbox { remote_id: id, name: display, path: name, role, is_label: true });
        }
        Ok(out)
    }

    async fn draft_id_for(&self, message_remote_id: &str) -> Result<Option<String>> {
        let mut token: Option<String> = None;
        loop {
            let mut url = format!("{GMAIL}/drafts?maxResults=500");
            if let Some(t) = &token {
                url.push_str(&format!("&pageToken={t}"));
            }
            let v = self.get(&url, "list drafts").await?;
            for d in v["drafts"].as_array().cloned().unwrap_or_default() {
                if d["message"]["id"].as_str() == Some(message_remote_id) {
                    return Ok(d["id"].as_str().map(String::from));
                }
            }
            token = v["nextPageToken"].as_str().map(String::from);
            if token.is_none() {
                return Ok(None);
            }
        }
    }

    async fn modify(&self, ids: Vec<String>, add: Vec<String>, remove: Vec<String>) -> Result<()> {
        for chunk in ids.chunks(1000) {
            self.post(&format!("{GMAIL}/messages/batchModify"), json!({ "ids": chunk, "addLabelIds": add, "removeLabelIds": remove }), "modify labels").await?;
        }
        Ok(())
    }
}

#[async_trait]
impl MailProvider for GmailProvider {
    async fn test(&self) -> Result<()> {
        self.get(&format!("{GMAIL}/profile"), "profile").await?;
        Ok(())
    }

    fn uses_labels(&self) -> bool {
        true
    }

    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let mut mbs = self.labels().await?;
        // Archived messages keep a membership in a virtual All Mail mailbox.
        mbs.push(RemoteMailbox { remote_id: "ALL".into(), name: "All Mail".into(), path: "All Mail".into(), role: MailboxRole::All, is_label: true });
        ctx.set_mailboxes(&mbs)?;
        let labels: HashSet<String> = mbs.iter().map(|m| m.remote_id.clone()).collect();
        match ctx.cursor("gmail:history")? {
            Some(h) => {
                if !self.delta(ctx, &h, &labels).await? {
                    debug!("Gmail history expired; full resync");
                    self.full_sync(ctx, &labels).await?;
                }
            }
            None => self.full_sync(ctx, &labels).await?,
        }
        if let Err(e) = self.backfill(ctx, &labels).await {
            warn!("Gmail backfill: {e:#}");
        }
        Ok(())
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let v = self.get(&format!("{GMAIL}/messages/{}?format=raw", msg.remote_id), "get raw message").await?;
        b64url_decode(v["raw"].as_str().context("no raw content")?)
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        self.apply_op(op).await
    }
}
