//! JMAP mail provider (RFC 8620/8621): Fastmail, Stalwart, Cyrus.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CredentialHandle, MailProvider, RemoteMailbox, ResolvedOp, SyncCtx};
use satchel_core::store::{MessageRecord, MessageRef};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tracing::{debug, warn};

const CAPS: [&str; 3] = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:ietf:params:jmap:submission"];
const EMAIL_PROPS: [&str; 19] = [
    "id", "threadId", "mailboxIds", "keywords", "receivedAt", "size", "messageId", "inReplyTo", "references", "subject", "from", "to", "cc", "bcc",
    "replyTo", "sentAt", "preview", "hasAttachment", "blobId",
];

#[derive(Clone, Debug)]
struct Session {
    api_url: String,
    download_url: String,
    upload_url: String,
    event_source_url: Option<String>,
    account_id: String,
    max_get: usize,
}

pub struct JmapProvider {
    session_url: String,
    username: String,
    creds: CredentialHandle,
    http: reqwest::Client,
    session: Mutex<Option<Session>>,
}

/// Servers sometimes advertise their public hostname; on loopback test servers
/// rebase those URLs onto the session URL's origin.
fn rebase(url: &str, session_url: &str) -> String {
    let (Ok(u), Ok(s)) = (url::Url::parse(url), url::Url::parse(session_url)) else { return url.to_string() };
    let loopback = |h: Option<&str>| matches!(h, Some("localhost") | Some("127.0.0.1") | Some("::1"));
    if loopback(s.host_str()) && loopback(u.host_str()) && u.origin() != s.origin() {
        let path = &url[u.origin().ascii_serialization().len()..];
        return format!("{}{}", s.origin().ascii_serialization(), path);
    }
    url.to_string()
}

fn addresses(v: &Value) -> Vec<Address> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| Some(Address { name: x["name"].as_str().filter(|s| !s.is_empty()).map(String::from), email: x["email"].as_str()?.to_string() }))
                .collect()
        })
        .unwrap_or_default()
}

fn first_str(v: &Value) -> Option<String> {
    v.as_array().and_then(|a| a.first()).and_then(Value::as_str).map(String::from)
}

fn flags_of(v: &Value) -> Flags {
    let k = &v["keywords"];
    Flags { seen: k["$seen"] == true, flagged: k["$flagged"] == true, answered: k["$answered"] == true, draft: k["$draft"] == true }
}

fn role_of(v: &Value) -> MailboxRole {
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

fn record(e: &Value) -> MessageRecord {
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
    }
}

fn mailboxes_of(e: &Value) -> Vec<(String, Option<u32>)> {
    e["mailboxIds"].as_object().map(|m| m.iter().filter(|(_, v)| **v == true).map(|(k, _)| (k.clone(), None)).collect()).unwrap_or_default()
}

impl JmapProvider {
    pub fn new(session_url: String, username: String, allow_invalid: bool, creds: CredentialHandle) -> Result<Self> {
        Ok(Self { session_url, username, creds, http: crate::net::http_client(allow_invalid)?, session: Mutex::new(None) })
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        Ok(match self.creds.get()? {
            Credential::Password { password } if password.starts_with("fmu1-") => rb.bearer_auth(password),
            Credential::Password { password } => rb.basic_auth(&self.username, Some(password)),
            Credential::OAuth { access_token, .. } => rb.bearer_auth(access_token),
        })
    }

    async fn session(&self) -> Result<Session> {
        if let Some(s) = self.session.lock().await.clone() {
            return Ok(s);
        }
        let resp = self.auth(self.http.get(&self.session_url))?.send().await.context("JMAP session request failed")?;
        let st = resp.status();
        let body = resp.text().await?;
        if !st.is_success() {
            return Err(crate::net::http_error(st, &body, "JMAP session"));
        }
        let v: Value = serde_json::from_str(&body)?;
        let account_id = v["primaryAccounts"]["urn:ietf:params:jmap:mail"].as_str().context("server has no JMAP mail account")?.to_string();
        let r = |k: &str| v[k].as_str().map(|u| rebase(u, &self.session_url));
        let s = Session {
            api_url: r("apiUrl").context("no apiUrl")?,
            download_url: r("downloadUrl").context("no downloadUrl")?,
            upload_url: r("uploadUrl").context("no uploadUrl")?,
            event_source_url: r("eventSourceUrl"),
            account_id,
            max_get: v["capabilities"]["urn:ietf:params:jmap:core"]["maxObjectsInGet"].as_u64().unwrap_or(256).min(500) as usize,
        };
        *self.session.lock().await = Some(s.clone());
        Ok(s)
    }

    /// Run method calls; returns responses by call id. Errors on any method error.
    async fn call(&self, calls: Vec<Value>) -> Result<HashMap<String, Value>> {
        let s = self.session().await?;
        let body = json!({ "using": CAPS, "methodCalls": calls });
        let resp = self.auth(self.http.post(&s.api_url).json(&body))?.send().await.context("JMAP request failed")?;
        let st = resp.status();
        let text = resp.text().await?;
        if !st.is_success() {
            if st.as_u16() == 401 {
                *self.session.lock().await = None;
            }
            return Err(crate::net::http_error(st, &text, "JMAP"));
        }
        let v: Value = serde_json::from_str(&text)?;
        let mut out = HashMap::new();
        for r in v["methodResponses"].as_array().cloned().unwrap_or_default() {
            let name = r[0].as_str().unwrap_or_default();
            let id = r[2].as_str().unwrap_or_default().to_string();
            if name == "error" {
                let t = r[1]["type"].as_str().unwrap_or("error");
                let err = anyhow::Error::new(ApiError::new(
                    if t == "cannotCalculateChanges" { ErrorCode::Conflict } else { ErrorCode::Internal },
                    format!("JMAP {t}: {}", r[1]["description"].as_str().unwrap_or_default()),
                ));
                return Err(err.context(format!("call {id}")));
            }
            out.insert(id, r[1].clone());
        }
        Ok(out)
    }

    async fn mailboxes(&self, acc: &str) -> Result<(Vec<RemoteMailbox>, HashMap<MailboxRole, String>, String)> {
        let r = self.call(vec![json!(["Mailbox/get", { "accountId": acc, "properties": ["name", "role", "parentId"] }, "m"])]).await?;
        let m = &r["m"];
        let list = m["list"].as_array().cloned().unwrap_or_default();
        let by_id: HashMap<String, &Value> = list.iter().map(|x| (x["id"].as_str().unwrap_or_default().to_string(), x)).collect();
        let path_of = |id: &str| {
            let mut parts = vec![];
            let mut cur = by_id.get(id).copied();
            let mut guard = 0;
            while let Some(c) = cur {
                parts.push(c["name"].as_str().unwrap_or_default().to_string());
                cur = c["parentId"].as_str().and_then(|p| by_id.get(p).copied());
                guard += 1;
                if guard > 20 {
                    break;
                }
            }
            parts.reverse();
            parts.join("/")
        };
        let mut roles = HashMap::new();
        let mut out = vec![];
        for x in &list {
            let id = x["id"].as_str().unwrap_or_default().to_string();
            let role = role_of(x);
            if role != MailboxRole::Custom {
                roles.entry(role).or_insert(id.clone());
            }
            let name = if role == MailboxRole::Inbox { "Inbox".into() } else { x["name"].as_str().unwrap_or_default().to_string() };
            out.push(RemoteMailbox { path: path_of(&id), remote_id: id, name, role, is_label: false });
        }
        Ok((out, roles, m["state"].as_str().unwrap_or_default().to_string()))
    }

    async fn get_emails(&self, ctx: &SyncCtx, acc: &str, ids: &[String]) -> Result<()> {
        let s = self.session().await?;
        for chunk in ids.chunks(s.max_get.max(1)) {
            let r = self.call(vec![json!(["Email/get", { "accountId": acc, "ids": chunk, "properties": EMAIL_PROPS }, "g"])]).await?;
            for e in r["g"]["list"].as_array().cloned().unwrap_or_default() {
                let mbs = mailboxes_of(&e);
                ctx.put_record(record(&e), &mbs)?;
            }
            for gone in r["g"]["notFound"].as_array().cloned().unwrap_or_default() {
                if let Some(id) = gone.as_str() {
                    ctx.delete_remote(id)?;
                }
            }
        }
        Ok(())
    }

    async fn full_sync(&self, ctx: &SyncCtx, acc: &str) -> Result<()> {
        // Page through every email id, newest first.
        let mut ids: Vec<String> = vec![];
        let mut position = 0usize;
        let mut state = String::new();
        loop {
            let r = self
                .call(vec![json!(["Email/query", { "accountId": acc, "sort": [{ "property": "receivedAt", "isAscending": false }], "position": position, "limit": 1000, "calculateTotal": true }, "q"])])
                .await?;
            let q = &r["q"];
            if state.is_empty() {
                state = q["queryState"].as_str().unwrap_or_default().to_string();
            }
            let page: Vec<String> = q["ids"].as_array().cloned().unwrap_or_default().into_iter().filter_map(|v| v.as_str().map(String::from)).collect();
            let n = page.len();
            ids.extend(page);
            position += n;
            ctx.progress(format!("Listing messages ({})", ids.len()));
            if n == 0 || q["total"].as_u64().is_some_and(|t| position as u64 >= t) {
                break;
            }
        }
        // State to resume deltas from.
        let st = self.call(vec![json!(["Email/get", { "accountId": acc, "ids": [], "properties": ["id"] }, "s"])]).await?;
        let email_state = st["s"]["state"].as_str().unwrap_or_default().to_string();
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let remote: std::collections::HashSet<&String> = ids.iter().collect();
        for (rid, local) in &known {
            if !remote.contains(rid) {
                ctx.store.delete_message(local)?;
            }
        }
        let missing: Vec<String> = ids.iter().filter(|i| !known.contains_key(*i)).cloned().collect();
        let existing: Vec<String> = ids.iter().filter(|i| known.contains_key(*i)).cloned().collect();
        let total = missing.len();
        for (i, chunk) in missing.chunks(500).enumerate() {
            ctx.progress(format!("Downloading headers {}/{}", i * 500, total));
            self.get_emails(ctx, acc, chunk).await?;
        }
        // Refresh flags/mailboxes of messages we already had.
        for chunk in existing.chunks(500) {
            let r = self.call(vec![json!(["Email/get", { "accountId": acc, "ids": chunk, "properties": ["id", "mailboxIds", "keywords"] }, "f"])]).await?;
            for e in r["f"]["list"].as_array().cloned().unwrap_or_default() {
                let rid = e["id"].as_str().unwrap_or_default();
                ctx.set_flags_remote(rid, flags_of(&e))?;
                ctx.set_memberships_remote(rid, &mailboxes_of(&e))?;
            }
        }
        ctx.set_cursor("jmap:email_state", &email_state)?;
        Ok(())
    }

    async fn delta_sync(&self, ctx: &SyncCtx, acc: &str, since: &str) -> Result<()> {
        let mut since = since.to_string();
        loop {
            let r = self.call(vec![json!(["Email/changes", { "accountId": acc, "sinceState": since, "maxChanges": 500 }, "c"])]).await?;
            let c = &r["c"];
            let ids = |k: &str| -> Vec<String> { c[k].as_array().cloned().unwrap_or_default().into_iter().filter_map(|v| v.as_str().map(String::from)).collect() };
            let mut changed = ids("created");
            changed.extend(ids("updated"));
            for d in ids("destroyed") {
                ctx.delete_remote(&d)?;
            }
            if !changed.is_empty() {
                self.get_emails(ctx, acc, &changed).await?;
            }
            since = c["newState"].as_str().unwrap_or(&since).to_string();
            ctx.set_cursor("jmap:email_state", &since)?;
            if c["hasMoreChanges"] != true {
                break;
            }
        }
        Ok(())
    }

    async fn upload(&self, raw: &[u8]) -> Result<String> {
        let s = self.session().await?;
        let url = s.upload_url.replace("{accountId}", &s.account_id);
        let resp = self.auth(self.http.post(&url).header("Content-Type", "message/rfc822").body(raw.to_vec()))?.send().await?;
        let st = resp.status();
        let t = resp.text().await?;
        if !st.is_success() {
            return Err(crate::net::http_error(st, &t, "JMAP upload"));
        }
        let v: Value = serde_json::from_str(&t)?;
        Ok(v["blobId"].as_str().context("upload returned no blobId")?.to_string())
    }

    async fn import(&self, acc: &str, raw: &[u8], mailbox: &str, keywords: Value) -> Result<String> {
        let blob = self.upload(raw).await?;
        let r = self
            .call(vec![json!(["Email/import", { "accountId": acc, "emails": { "e": { "blobId": blob, "mailboxIds": { mailbox: true }, "keywords": keywords } } }, "i"])])
            .await?;
        if let Some(err) = r["i"]["notCreated"]["e"].as_object() {
            bail!("import failed: {}", serde_json::to_string(err)?);
        }
        Ok(r["i"]["created"]["e"]["id"].as_str().context("import returned no id")?.to_string())
    }

    async fn role_mailbox(&self, acc: &str, role: MailboxRole) -> Result<Option<String>> {
        Ok(self.mailboxes(acc).await?.1.get(&role).cloned())
    }

    async fn identity(&self, acc: &str, from: &str) -> Result<String> {
        let r = self.call(vec![json!(["Identity/get", { "accountId": acc }, "id"])]).await?;
        let list = r["id"]["list"].as_array().cloned().unwrap_or_default();
        let pick = list
            .iter()
            .find(|i| i["email"].as_str().is_some_and(|e| e.eq_ignore_ascii_case(from)))
            .or(list.first())
            .context("no sending identity on this account")?;
        Ok(pick["id"].as_str().unwrap_or_default().to_string())
    }

    fn update_all(&self, acc: &str, messages: &[MessageRef], patch: Value) -> Value {
        let mut upd = serde_json::Map::new();
        for m in messages {
            upd.insert(m.remote_id.clone(), patch.clone());
        }
        json!(["Email/set", { "accountId": acc, "update": upd }, "u"])
    }

    fn check_set(r: &HashMap<String, Value>, id: &str) -> Result<()> {
        for k in ["notUpdated", "notDestroyed", "notCreated"] {
            if let Some(m) = r[id][k].as_object()
                && !m.is_empty() {
                    bail!("JMAP {k}: {}", serde_json::to_string(m)?);
                }
        }
        Ok(())
    }
}

#[async_trait]
impl MailProvider for JmapProvider {
    async fn test(&self) -> Result<()> {
        let s = self.session().await?;
        self.call(vec![json!(["Mailbox/get", { "accountId": s.account_id, "ids": [] }, "t"])]).await?;
        Ok(())
    }

    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let acc = self.session().await?.account_id;
        let (mbs, _, _) = self.mailboxes(&acc).await?;
        ctx.set_mailboxes(&mbs)?;
        match ctx.cursor("jmap:email_state")? {
            Some(st) if !st.is_empty() => match self.delta_sync(ctx, &acc, &st).await {
                Ok(()) => Ok(()),
                Err(e) if e.chain().any(|c| c.to_string().contains("cannotCalculateChanges")) => {
                    debug!("JMAP state too old; full resync");
                    self.full_sync(ctx, &acc).await
                }
                Err(e) => Err(e),
            },
            _ => self.full_sync(ctx, &acc).await,
        }
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let s = self.session().await?;
        let r = self.call(vec![json!(["Email/get", { "accountId": s.account_id, "ids": [msg.remote_id], "properties": ["blobId"] }, "b"])]).await?;
        let blob = r["b"]["list"][0]["blobId"].as_str().ok_or_else(|| anyhow::Error::new(ApiError::new(ErrorCode::NotFound, "message no longer on server")))?;
        let url = s
            .download_url
            .replace("{accountId}", &s.account_id)
            .replace("{blobId}", blob)
            .replace("{name}", "message.eml")
            .replace("{type}", "message/rfc822");
        let resp = self.auth(self.http.get(&url))?.send().await?;
        let st = resp.status();
        if !st.is_success() {
            return Err(crate::net::http_error(st, &resp.text().await.unwrap_or_default(), "JMAP download"));
        }
        Ok(resp.bytes().await?.to_vec())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let acc = self.session().await?.account_id;
        let mut out = ApplyOutcome::default();
        match op {
            ResolvedOp::SetFlags { messages, seen, flagged, answered } => {
                let mut patch = serde_json::Map::new();
                for (v, k) in [(seen, "$seen"), (flagged, "$flagged"), (answered, "$answered")] {
                    if let Some(b) = v {
                        patch.insert(format!("keywords/{k}"), if *b { json!(true) } else { Value::Null });
                    }
                }
                let r = self.call(vec![self.update_all(&acc, messages, Value::Object(patch))]).await?;
                Self::check_set(&r, "u")?;
            }
            ResolvedOp::Move { messages, to_remote_mailbox } => {
                let r = self.call(vec![self.update_all(&acc, messages, json!({ "mailboxIds": { to_remote_mailbox: true } }))]).await?;
                Self::check_set(&r, "u")?;
                for m in messages {
                    out.rekeyed.push((m.id.clone(), m.remote_id.clone(), vec![(to_remote_mailbox.clone(), None)]));
                }
            }
            ResolvedOp::Labels { messages, add, remove } => {
                let mut patch = serde_json::Map::new();
                for a in add {
                    patch.insert(format!("mailboxIds/{a}"), json!(true));
                }
                for r in remove {
                    patch.insert(format!("mailboxIds/{r}"), Value::Null);
                }
                let r = self.call(vec![self.update_all(&acc, messages, Value::Object(patch))]).await?;
                Self::check_set(&r, "u")?;
            }
            ResolvedOp::Expunge { messages } => {
                let ids: Vec<&String> = messages.iter().map(|m| &m.remote_id).collect();
                let r = self.call(vec![json!(["Email/set", { "accountId": acc, "destroy": ids }, "d"])]).await?;
                Self::check_set(&r, "d")?;
            }
            ResolvedOp::SaveDraft { raw, replaces, drafts_remote_mailbox } => {
                let mb = match drafts_remote_mailbox {
                    Some(m) => m.clone(),
                    None => self.role_mailbox(&acc, MailboxRole::Drafts).await?.context("no Drafts mailbox")?,
                };
                let id = self.import(&acc, raw, &mb, json!({ "$draft": true, "$seen": true })).await?;
                out.stored_remote_id = Some(id);
                if let Some(old) = replaces {
                    let _ = self.call(vec![json!(["Email/set", { "accountId": acc, "destroy": [old.remote_id] }, "d"])]).await;
                }
            }
            ResolvedOp::DeleteDraft { message } => {
                let r = self.call(vec![json!(["Email/set", { "accountId": acc, "destroy": [message.remote_id] }, "d"])]).await?;
                Self::check_set(&r, "d")?;
            }
            ResolvedOp::Send { raw, from, rcpts, sent_remote_mailbox, replaces_draft, .. } => {
                let drafts = self.role_mailbox(&acc, MailboxRole::Drafts).await?;
                let sent = match sent_remote_mailbox {
                    Some(s) => Some(s.clone()),
                    None => self.role_mailbox(&acc, MailboxRole::Sent).await?,
                };
                let staging = drafts.clone().or(sent.clone()).context("no Drafts or Sent mailbox to stage the message")?;
                let email_id = self.import(&acc, raw, &staging, json!({ "$draft": true, "$seen": true })).await?;
                let identity = self.identity(&acc, from).await?;
                let mut on_success = serde_json::Map::new();
                on_success.insert("keywords/$draft".into(), Value::Null);
                if let Some(s) = &sent {
                    on_success.insert(format!("mailboxIds/{s}"), json!(true));
                    if &staging != s {
                        on_success.insert(format!("mailboxIds/{staging}"), Value::Null);
                    }
                }
                let envelope = json!({ "mailFrom": { "email": from }, "rcptTo": rcpts.iter().map(|r| json!({ "email": r })).collect::<Vec<_>>() });
                let r = self
                    .call(vec![json!(["EmailSubmission/set", {
                        "accountId": acc,
                        "create": { "s": { "emailId": email_id, "identityId": identity, "envelope": envelope } },
                        "onSuccessUpdateEmail": { "#s": on_success }
                    }, "sub"])])
                    .await?;
                if let Some(e) = r["sub"]["notCreated"]["s"].as_object() {
                    // Not sent: remove the staged copy so a retry doesn't duplicate it.
                    let _ = self.call(vec![json!(["Email/set", { "accountId": acc, "destroy": [email_id] }, "d"])]).await;
                    let desc = e.get("description").and_then(Value::as_str).unwrap_or("submission rejected");
                    return Err(anyhow::Error::new(ApiError::new(ErrorCode::InvalidInput, format!("send failed: {desc}"))));
                }
                if let Some(d) = replaces_draft
                    && let Err(e) = self.call(vec![json!(["Email/set", { "accountId": acc, "destroy": [d.remote_id] }, "d"])]).await {
                        warn!("could not remove draft: {e:#}");
                    }
            }
        }
        Ok(out)
    }

    async fn wait_for_changes(&self, _ctx: &SyncCtx, max_wait: Duration) -> Result<bool> {
        let s = self.session().await?;
        let Some(es) = s.event_source_url else {
            tokio::time::sleep(max_wait).await;
            return Ok(true);
        };
        let url = es.replace("{types}", "Email,Mailbox").replace("{closeafter}", "state").replace("{ping}", "60");
        let fut = async {
            let resp = self.auth(self.http.get(&url).header("Accept", "text/event-stream").timeout(max_wait + Duration::from_secs(5)))?.send().await?;
            if !resp.status().is_success() {
                return Err(anyhow!("event source HTTP {}", resp.status()));
            }
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                buf.push_str(&String::from_utf8_lossy(&chunk?));
                // A "state" event means something changed.
                if buf.contains("event: state") || buf.contains("event:state") {
                    return Ok(true);
                }
                if buf.len() > 64 * 1024 {
                    buf.clear();
                }
            }
            Ok(true)
        };
        match tokio::time::timeout(max_wait, fut).await {
            Ok(r) => r,
            Err(_) => Ok(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebases_loopback_only() {
        assert_eq!(rebase("https://localhost/jmap/", "http://127.0.0.1:8480/jmap/session"), "http://127.0.0.1:8480/jmap/");
        assert_eq!(rebase("https://api.fastmail.com/jmap/api/", "https://api.fastmail.com/jmap/session"), "https://api.fastmail.com/jmap/api/");
        assert_eq!(rebase("https://localhost/jmap/download/{accountId}/{blobId}/{name}?accept={type}", "http://127.0.0.1:8480/s"), "http://127.0.0.1:8480/jmap/download/{accountId}/{blobId}/{name}?accept={type}");
    }

    #[test]
    fn converts_email() {
        let e = json!({ "id": "M1", "threadId": "T1", "mailboxIds": { "a": true, "b": false }, "keywords": { "$seen": true },
            "receivedAt": "2026-01-02T03:04:05Z", "messageId": ["x@y"], "subject": "Hi", "from": [{ "name": "Bob", "email": "b@x" }], "preview": "hello" });
        let r = record(&e);
        assert_eq!(r.remote_thread_id.as_deref(), Some("T1"));
        assert_eq!(r.message_id_hdr.as_deref(), Some("x@y"));
        assert!(r.flags.seen && !r.flags.flagged);
        assert_eq!(mailboxes_of(&e), vec![("a".to_string(), None)]);
    }
}
