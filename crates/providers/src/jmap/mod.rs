//! JMAP mail provider (RFC 8620/8621): Fastmail, Stalwart, Cyrus.

mod apply;
mod convert;
mod parts;
mod sync;
#[cfg(test)]
mod tests;

use convert::*;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CredentialHandle, Download, MailProvider, RemoteMailbox, ResolvedOp, SyncCtx};
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
        let body = crate::net::text_capped(resp, crate::net::API_LIMIT).await?;
        if !st.is_success() {
            return Err(crate::net::http_error(st, &body, "JMAP session"));
        }
        let v: Value = serde_json::from_str(&body)?;
        let account_id = v["primaryAccounts"]["urn:ietf:params:jmap:mail"].as_str().context("server has no JMAP mail account")?.to_string();
        let r = |k: &str, kind: Use| v[k].as_str().map(|u| endpoint(&rebase(u, &self.session_url), &self.session_url, kind)).transpose();
        let s = Session {
            api_url: r("apiUrl", Use::Api)?.context("no apiUrl")?,
            download_url: r("downloadUrl", Use::Download)?.context("no downloadUrl")?,
            upload_url: r("uploadUrl", Use::Api)?.context("no uploadUrl")?,
            event_source_url: r("eventSourceUrl", Use::Api)?,
            account_id,
            max_get: v["capabilities"]["urn:ietf:params:jmap:core"]["maxObjectsInGet"].as_u64().unwrap_or(256).min(500) as usize,
        };
        *self.session.lock().await = Some(s.clone());
        Ok(s)
    }

    /// Run method calls; returns responses by call id. Errors on any method error.
    async fn call(&self, calls: Vec<Value>) -> Result<HashMap<String, Value>> {
        satchel_core::provider::priority::yield_to_user().await;
        let s = self.session().await?;
        let body = json!({ "using": CAPS, "methodCalls": calls });
        let resp = self.auth(self.http.post(&s.api_url).json(&body))?.send().await.context("JMAP request failed")?;
        let st = resp.status();
        let text = crate::net::text_capped(resp, crate::net::DATA_LIMIT).await?;
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
                    // Method errors mean the method didn't run; only server
                    // trouble is worth retrying.
                    match t {
                        "cannotCalculateChanges" => ErrorCode::Conflict,
                        "serverFail" | "serverPartialFail" | "serverUnavailable" => ErrorCode::Network,
                        "forbidden" | "accountReadOnly" => ErrorCode::Forbidden,
                        _ => ErrorCode::InvalidInput,
                    },
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

    /// Download a blob's contents.
    async fn download(&self, s: &Session, blob: &str, ty: &str) -> Result<Vec<u8>> {
        // The URL is a URI template (RFC 8620 §2): values are percent-encoded.
        let enc = crate::net::enc;
        let url = s
            .download_url
            .replace("{accountId}", &enc(&s.account_id))
            .replace("{blobId}", &enc(blob))
            .replace("{name}", if ty == "message/rfc822" { "message.eml" } else { "part" })
            .replace("{type}", &enc(ty));
        let resp = self.auth(self.http.get(&url))?.send().await?;
        let st = resp.status();
        if !st.is_success() {
            return Err(crate::net::http_error(st, &crate::net::error_text(resp).await, "JMAP download"));
        }
        crate::net::body_capped(resp, crate::net::DOWNLOAD_LIMIT).await
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
                Ok(()) => {}
                Err(e) if e.chain().any(|c| c.to_string().contains("cannotCalculateChanges")) => {
                    debug!("JMAP state too old; full resync");
                    self.full_sync(ctx, &acc).await?;
                }
                Err(e) => return Err(e),
            },
            _ => self.full_sync(ctx, &acc).await?,
        }
        if let Err(e) = self.backfill(ctx, &acc).await {
            warn!("JMAP older mail: {e:#}");
        }
        Ok(())
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let s = self.session().await?;
        let r = self.call(vec![json!(["Email/get", { "accountId": s.account_id, "ids": [msg.remote_id], "properties": ["blobId"] }, "b"])]).await?;
        let blob = r["b"]["list"][0]["blobId"].as_str().ok_or_else(|| anyhow::Error::new(ApiError::new(ErrorCode::NotFound, "message no longer on server")))?;
        self.download(&s, blob, "message/rfc822").await
    }

    async fn fetch_without_attachments(&self, ctx: &SyncCtx, msg: &MessageRef) -> Result<Download> {
        match self.fetch_slim(&msg.remote_id).await? {
            Some(slim) => Ok(Download::Slim(slim)),
            None => Ok(Download::Full(self.fetch_raw(ctx, msg).await?)),
        }
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        self.apply_op(op).await
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
