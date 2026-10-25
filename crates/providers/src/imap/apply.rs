//! Applying ops: flags, moves, expunge, drafts, folders, sending.

use super::*;

impl ImapProvider {
    /// Group message refs by mailbox: (uid, UIDVALIDITY the uid belongs to,
    /// when the ref records it).
    pub(super) fn by_mailbox(messages: &[MessageRef]) -> HashMap<String, Vec<(u32, Option<u32>)>> {
        let mut out: HashMap<String, Vec<(u32, Option<u32>)>> = HashMap::new();
        for m in messages {
            if let Some((uv, uid, mb)) = parse_remote_id(&m.remote_id) {
                out.entry(mb.to_string()).or_default().push((uid, Some(uv)));
                continue;
            }
            for (_, rmb, uid) in &m.mailboxes {
                if let Some(u) = uid {
                    out.entry(rmb.clone()).or_default().push((*u, None));
                }
            }
        }
        out
    }

    /// Select `mailbox` and return the uids still valid in it. A uid from
    /// before a UIDVALIDITY change (the server renumbered the folder) now
    /// names some other message, so it is dropped rather than acted on.
    pub(super) async fn current_uids(&self, s: &mut Session, mailbox: &str, items: &[(u32, Option<u32>)]) -> Result<Vec<u32>> {
        let sel = s.select(mailbox).await?;
        let now = sel.uid_validity;
        let (ok, stale): (Vec<_>, Vec<_>) = items.iter().partition(|(_, uv)| uv.is_none() || now.is_none() || *uv == now);
        if !stale.is_empty() {
            warn!(mailbox, stale = stale.len(), "skipping messages from before the folder was renumbered (UIDVALIDITY changed)");
        }
        Ok(ok.into_iter().map(|(u, _)| u).collect())
    }

    /// UIDNEXT of a mailbox (taken before a MOVE/APPEND so we can find the new copies).
    pub(super) async fn uid_next(&self, s: &mut Session, mailbox: &str) -> Result<(u32, u32)> {
        let sel = s.select(mailbox).await?;
        Ok((sel.uid_validity.unwrap_or(0), sel.uid_next.unwrap_or(1)))
    }

    /// Message-ID -> (uidvalidity, uid) for messages that arrived at or after `from_uid`.
    pub(super) async fn locate_new(&self, s: &mut Session, mailbox: &str, from_uid: u32) -> Result<HashMap<String, (u32, u32)>> {
        let sel = s.select(mailbox).await?;
        let uv = sel.uid_validity.unwrap_or(0);
        let mut out = HashMap::new();
        if sel.exists == 0 {
            return Ok(out);
        }
        let fetches: Vec<Fetch> = s.uid_fetch(format!("{from_uid}:*"), "(UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])").await?.try_collect().await?;
        for f in fetches {
            let (Some(uid), Some(h)) = (f.uid, f.header().or(f.body())) else { continue };
            if uid < from_uid {
                continue; // "n:*" always returns the last message even if older
            }
            if let Some(mid) = satchel_mime::parse(h).and_then(|p| p.message_id) {
                out.insert(mid, (uv, uid));
            }
        }
        Ok(out)
    }

    pub(super) async fn expunge_uids(&self, s: &mut Session, mailbox: &str, uids: &[u32]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        s.select(mailbox).await?;
        let set = uid_set(uids);
        if self.has_cap("UIDPLUS").await {
            let _: Vec<Fetch> = s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
            let _: Vec<u32> = s.uid_expunge(&set).await?.try_collect().await?;
            return Ok(());
        }
        // Plain EXPUNGE removes every \Deleted message in the mailbox, including
        // ones another client flagged. Unflag those around it (RFC 4315 §1).
        let ours: HashSet<u32> = uids.iter().copied().collect();
        let others: Vec<u32> = s.uid_search("DELETED").await?.into_iter().filter(|u| !ours.contains(u)).collect();
        if !others.is_empty() {
            let _: Vec<Fetch> = s.uid_store(uid_set(&others), "-FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
        }
        let expunged = async {
            let _: Vec<Fetch> = s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
            let _: Vec<u32> = s.expunge().await?.try_collect().await?;
            anyhow::Ok(())
        }
        .await;
        if !others.is_empty() {
            let _: Vec<Fetch> = s.uid_store(uid_set(&others), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
        }
        expunged
    }
    /// The server name for folder `name` under `parent` (a raw server name),
    /// plus the hierarchy delimiter. Top-level folders go under "INBOX." on
    /// servers that keep everything there.
    pub(super) async fn folder_name(&self, s: &mut Session, parent: Option<&str>, name: &str) -> Result<(String, String)> {
        let names: Vec<async_imap::types::Name> = s.list(Some(""), Some("*")).await?.try_collect().await?;
        let delim = parent
            .and_then(|p| names.iter().find(|n| n.name() == p))
            .or_else(|| names.first())
            .and_then(|n| n.delimiter())
            .unwrap_or("/")
            .to_string();
        if name.contains(delim.as_str()) {
            return Err(ApiError::new(ErrorCode::InvalidInput, format!("this server doesn't allow \"{delim}\" in folder names")).into());
        }
        let prefix = match parent {
            Some(p) => format!("{p}{delim}"),
            None => {
                let inbox = format!("INBOX{delim}");
                let others: Vec<&str> = names.iter().map(|n| n.name()).filter(|n| !n.eq_ignore_ascii_case("INBOX")).collect();
                if !others.is_empty() && others.iter().all(|n| n.starts_with(&inbox)) { inbox } else { String::new() }
            }
        };
        Ok((format!("{prefix}{}", encode_mutf7(name)), delim))
    }

    pub(super) async fn apply_inner(&self, s: &mut Session, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            ResolvedOp::SetFlags { messages, seen, flagged, answered } => {
                for (mb, items) in Self::by_mailbox(messages) {
                    let uids = self.current_uids(s, &mb, &items).await?;
                    if uids.is_empty() {
                        continue;
                    }
                    let set = uid_set(&uids);
                    let mut add = vec![];
                    let mut rem = vec![];
                    for (v, flag) in [(seen, "\\Seen"), (flagged, "\\Flagged"), (answered, "\\Answered")] {
                        match v {
                            Some(true) => add.push(flag),
                            Some(false) => rem.push(flag),
                            None => {}
                        }
                    }
                    if !add.is_empty() {
                        let _: Vec<Fetch> = s.uid_store(&set, format!("+FLAGS.SILENT ({})", add.join(" "))).await?.try_collect().await?;
                    }
                    if !rem.is_empty() {
                        let _: Vec<Fetch> = s.uid_store(&set, format!("-FLAGS.SILENT ({})", rem.join(" "))).await?.try_collect().await?;
                    }
                }
            }
            ResolvedOp::Move { messages, to_remote_mailbox } => {
                let has_move = self.has_cap("MOVE").await;
                let (_, next_before) = self.uid_next(s, to_remote_mailbox).await?;
                for (mb, items) in Self::by_mailbox(messages) {
                    if &mb == to_remote_mailbox {
                        continue;
                    }
                    let uids = self.current_uids(s, &mb, &items).await?;
                    if uids.is_empty() {
                        continue;
                    }
                    let set = uid_set(&uids);
                    if has_move {
                        s.uid_mv(&set, to_remote_mailbox).await?;
                    } else {
                        s.uid_copy(&set, to_remote_mailbox).await?;
                        self.expunge_uids(s, &mb, &uids).await?;
                    }
                }
                // Re-identify moved messages in the target mailbox so local ids survive.
                let arrived = self.locate_new(s, to_remote_mailbox, next_before).await?;
                for m in messages {
                    if let Some((uv, uid)) = m.message_id_hdr.as_ref().and_then(|mid| arrived.get(mid)) {
                        out.rekeyed.push((m.id.clone(), remote_id(*uv, *uid, to_remote_mailbox), vec![(to_remote_mailbox.clone(), Some(*uid))]));
                    }
                }
            }
            ResolvedOp::Labels { .. } => bail!("IMAP accounts use folders, not labels"),
            ResolvedOp::Expunge { messages } => {
                for (mb, items) in Self::by_mailbox(messages) {
                    let uids = self.current_uids(s, &mb, &items).await?;
                    self.expunge_uids(s, &mb, &uids).await?;
                }
            }
            ResolvedOp::SaveDraft { raw, replaces, drafts_remote_mailbox } => {
                let mb = drafts_remote_mailbox.clone().unwrap_or_else(|| "Drafts".into());
                let (_, next_before) = self.uid_next(s, &mb).await?;
                s.append(&mb, Some("(\\Draft \\Seen)"), None, raw).await?;
                if let Some(old) = replaces {
                    for (omb, items) in Self::by_mailbox(std::slice::from_ref(old)) {
                        let uids = self.current_uids(s, &omb, &items).await?;
                        self.expunge_uids(s, &omb, &uids).await?;
                    }
                }
                let parsed = satchel_mime::parse(raw).unwrap_or_default();
                if let Some(mid) = parsed.message_id
                    && let Some((uv, uid)) = self.locate_new(s, &mb, next_before).await?.get(&mid) {
                        out.stored_remote_id = Some(remote_id(*uv, *uid, &mb));
                    }
            }
            ResolvedOp::DeleteDraft { message } => {
                for (mb, items) in Self::by_mailbox(std::slice::from_ref(message)) {
                    let uids = self.current_uids(s, &mb, &items).await?;
                    self.expunge_uids(s, &mb, &uids).await?;
                }
            }
            ResolvedOp::CreateMailbox { name, parent, .. } => {
                let (raw, _) = self.folder_name(s, parent.as_deref(), name).await?;
                s.create(&raw).await?;
                // Not every server subscribes new folders, and some clients only show subscribed ones.
                let _ = s.subscribe(&raw).await;
                out.mailbox_remote_id = Some(raw);
            }
            ResolvedOp::UpdateMailbox { remote_id, name, parent, .. } => {
                let (raw, delim) = self.folder_name(s, parent.as_deref(), name).await?;
                if raw != *remote_id {
                    // Folders under it are renamed with it; work out their new names first.
                    let under = format!("{remote_id}{delim}");
                    let children: Vec<String> =
                        s.list(Some(""), Some(&format!("{under}*"))).await?.map_ok(|n| n.name().to_string()).try_collect().await?;
                    s.rename(remote_id, &raw).await?;
                    let _ = s.unsubscribe(remote_id).await;
                    let _ = s.subscribe(&raw).await;
                    out.mailbox_renamed.push((remote_id.clone(), raw.clone()));
                    for c in children {
                        if let Some(rest) = c.strip_prefix(&under) {
                            out.mailbox_renamed.push((c.clone(), format!("{raw}{delim}{rest}")));
                        }
                    }
                }
            }
            ResolvedOp::DeleteMailbox { remote_id, .. } => {
                // IMAP DELETE takes the folder's messages with it. The core
                // moves them out first; never delete a folder that still has any.
                let st = s.status(remote_id, "(MESSAGES)").await?;
                if st.exists > 0 {
                    return Err(ApiError::new(ErrorCode::Conflict, format!("not deleted: the folder still has {} message(s) on the server", st.exists)).into());
                }
                let _ = s.unsubscribe(remote_id).await;
                s.delete(remote_id).await?;
            }
            ResolvedOp::Send { raw, from, rcpts, sent_remote_mailbox, replaces_draft, .. } => {
                self.smtp_send(raw, from, rcpts).await?;
                // After this point the mail is out: never fail the op (it would resend).
                if !self.server_saves_sent()
                    && let Some(sent) = sent_remote_mailbox
                        && let Err(e) = s.append(sent, Some("(\\Seen)"), None, raw).await {
                            warn!("could not save to Sent: {e}");
                        }
                if let Some(d) = replaces_draft {
                    for (mb, items) in Self::by_mailbox(std::slice::from_ref(d)) {
                        let removed = async {
                            let uids = self.current_uids(s, &mb, &items).await?;
                            self.expunge_uids(s, &mb, &uids).await
                        };
                        if let Err(e) = removed.await {
                            warn!("could not remove draft: {e}");
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}
