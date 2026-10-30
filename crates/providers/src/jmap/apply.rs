//! Applying ops through Email/set, Email/import and EmailSubmission/set.

use super::*;

impl JmapProvider {
    pub(super) async fn upload(&self, raw: &[u8]) -> Result<String> {
        let s = self.session().await?;
        let url = s.upload_url.replace("{accountId}", &crate::net::enc(&s.account_id));
        let resp = self.auth(self.http.post(&url).header("Content-Type", "message/rfc822").body(raw.to_vec()))?.send().await?;
        let st = resp.status();
        let t = crate::net::text_capped(resp, crate::net::API_LIMIT).await?;
        if !st.is_success() {
            return Err(crate::net::http_error(st, &t, "JMAP upload"));
        }
        let v: Value = serde_json::from_str(&t)?;
        Ok(v["blobId"].as_str().context("upload returned no blobId")?.to_string())
    }

    pub(super) async fn import(&self, acc: &str, raw: &[u8], mailbox: &str, keywords: Value) -> Result<String> {
        let blob = self.upload(raw).await?;
        let r = self
            .call(vec![json!(["Email/import", { "accountId": acc, "emails": { "e": { "blobId": blob, "mailboxIds": { mailbox: true }, "keywords": keywords } } }, "i"])])
            .await?;
        if let Some(err) = r["i"]["notCreated"]["e"].as_object() {
            bail!("import failed: {}", serde_json::to_string(err)?);
        }
        Ok(r["i"]["created"]["e"]["id"].as_str().context("import returned no id")?.to_string())
    }

    /// Destroy a server draft, if it still is one. In JMAP a sent draft
    /// keeps its id (it just leaves Drafts and loses `$draft`), so a draft
    /// sent from another client must not be destroyed as "the old draft".
    pub(super) async fn destroy_draft(&self, acc: &str, id: &str) -> Result<()> {
        let r = self.call(vec![json!(["Email/get", { "accountId": acc, "ids": [id], "properties": ["keywords"] }, "g"])]).await?;
        let still_draft = r.get("g").and_then(|g| g["list"].as_array()).and_then(|l| l.first()).is_some_and(|e| e["keywords"]["$draft"] == true);
        if !still_draft {
            return Ok(());
        }
        let r = self.call(vec![json!(["Email/set", { "accountId": acc, "destroy": [id] }, "d"])]).await?;
        Self::check_set(&r, "d")
    }

    pub(super) async fn role_mailbox(&self, acc: &str, role: MailboxRole) -> Result<Option<String>> {
        Ok(self.mailboxes(acc).await?.1.get(&role).cloned())
    }

    pub(super) async fn identity(&self, acc: &str, from: &str) -> Result<String> {
        let r = self.call(vec![json!(["Identity/get", { "accountId": acc }, "id"])]).await?;
        let list = r["id"]["list"].as_array().cloned().unwrap_or_default();
        let pick = list
            .iter()
            .find(|i| i["email"].as_str().is_some_and(|e| e.eq_ignore_ascii_case(from)))
            .or(list.first())
            .context("no sending identity on this account")?;
        Ok(pick["id"].as_str().unwrap_or_default().to_string())
    }

    pub(super) fn update_all(&self, acc: &str, messages: &[MessageRef], patch: Value) -> Value {
        let mut upd = serde_json::Map::new();
        for m in messages {
            upd.insert(m.remote_id.clone(), patch.clone());
        }
        json!(["Email/set", { "accountId": acc, "update": upd }, "u"])
    }

    pub(super) fn check_set(r: &HashMap<String, Value>, id: &str) -> Result<()> {
        let res = r.get(id).with_context(|| format!("JMAP response {id} missing"))?;
        for k in ["notUpdated", "notDestroyed", "notCreated"] {
            // Already gone (deleted by another client) is done, not an error.
            if let Some(m) = res[k].as_object()
                && m.values().any(|e| e["type"] != "notFound")
            {
                bail!("JMAP {k}: {}", serde_json::to_string(m)?);
            }
        }
        Ok(())
    }

    pub(super) async fn apply_op(&self, op: &ResolvedOp) -> Result<ApplyOutcome> {
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
                    let _ = self.destroy_draft(&acc, &old.remote_id).await;
                }
            }
            ResolvedOp::DeleteDraft { message } => {
                self.destroy_draft(&acc, &message.remote_id).await?;
            }
            ResolvedOp::CreateMailbox { name, parent, .. } => {
                let r = self.call(vec![json!(["Mailbox/set", { "accountId": acc, "create": { "f": { "name": name, "parentId": parent } } }, "m"])]).await?;
                Self::check_set(&r, "m")?;
                out.mailbox_remote_id = Some(r["m"]["created"]["f"]["id"].as_str().context("Mailbox/set returned no id")?.to_string());
            }
            ResolvedOp::UpdateMailbox { remote_id, name, parent, .. } => {
                let r = self.call(vec![json!(["Mailbox/set", { "accountId": acc, "update": { remote_id: { "name": name, "parentId": parent } } }, "m"])]).await?;
                Self::check_set(&r, "m")?;
            }
            ResolvedOp::DeleteMailbox { remote_id, .. } => {
                // The core moved the messages out already; never take mail with it.
                let r = self.call(vec![json!(["Mailbox/set", { "accountId": acc, "destroy": [remote_id], "onDestroyRemoveEmails": false }, "m"])]).await?;
                Self::check_set(&r, "m")?;
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
                    .await
                    .map_err(crate::net::send_error)?;
                if let Some(e) = r["sub"]["notCreated"]["s"].as_object() {
                    // Not sent: remove the staged copy so a retry doesn't duplicate it.
                    let _ = self.call(vec![json!(["Email/set", { "accountId": acc, "destroy": [email_id] }, "d"])]).await;
                    let desc = e.get("description").and_then(Value::as_str).unwrap_or("submission rejected");
                    return Err(anyhow::Error::new(ApiError::new(ErrorCode::InvalidInput, format!("send failed: {desc}"))));
                }
                if let Some(d) = replaces_draft
                    && let Err(e) = self.destroy_draft(&acc, &d.remote_id).await {
                        warn!("could not remove draft: {e:#}");
                    }
            }
        }
        Ok(out)
    }
}
