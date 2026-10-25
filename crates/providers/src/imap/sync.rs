//! Listing folders and syncing each one.

use super::*;

impl ImapProvider {
    pub(super) async fn list_mailboxes(&self, s: &mut Session) -> Result<Vec<(RemoteMailbox, bool)>> {
        let names: Vec<async_imap::types::Name> = s.list(Some(""), Some("*")).await?.try_collect().await?;
        let mut out = vec![];
        for n in names {
            let selectable = !n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect))
                && !n.attributes().iter().any(|a| matches!(a, NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\NonExistent")));
            let raw = n.name().to_string();
            let decoded = decode_mutf7(&raw);
            let delim = n.delimiter().unwrap_or("/");
            let leaf = decoded.rsplit(delim).next().unwrap_or(&decoded).to_string();
            let role = role_for(&decoded, n.attributes());
            let display = if role == MailboxRole::Inbox { "Inbox".to_string() } else { leaf };
            out.push((
                RemoteMailbox { remote_id: raw, name: display, path: decoded.replace(delim, "/"), role, is_label: false },
                selectable,
            ));
        }
        Ok(out)
    }

    pub(super) async fn sync_mailbox(&self, s: &mut Session, ctx: &SyncCtx, mb: &RemoteMailbox) -> Result<()> {
        let condstore = self.has_cap("CONDSTORE").await;
        let sel = if condstore { s.select_condstore(&mb.remote_id).await? } else { s.select(&mb.remote_id).await? };
        let uv = sel.uid_validity.unwrap_or(0);
        let key = format!("imap:{}", mb.remote_id);
        let cursor: Option<MailboxCursor> = ctx.cursor(&key)?.and_then(|c| serde_json::from_str(&c).ok());
        let local_mb = ctx.mailbox_local_id(&mb.remote_id)?.context("mailbox not stored")?;
        let full = cursor.as_ref().is_none_or(|c| c.uidvalidity != uv);
        let unchanged = !full
            && cursor.as_ref().is_some_and(|c| sel.highest_modseq.is_some() && c.modseq == sel.highest_modseq && Some(c.uidnext) == sel.uid_next);
        if unchanged {
            return Ok(());
        }

        // Local view of this mailbox.
        let mut local: HashMap<u32, (String, Flags)> =
            ctx.store.mailbox_uids(&local_mb)?.into_iter().map(|(u, id, f)| (u, (id, f))).collect();
        if full && cursor.is_some() {
            // UIDVALIDITY changed: every UID is meaningless now.
            for (_, (id, _)) in local.drain() {
                ctx.store.remove_from_mailbox(&id, &local_mb)?;
            }
        }

        // Flags (all, or just changes since the last modseq).
        let since = (!full).then(|| cursor.as_ref().and_then(|c| c.modseq)).flatten().filter(|_| condstore);
        let query = match since {
            Some(m) => format!("(UID FLAGS) (CHANGEDSINCE {m})"),
            None => "(UID FLAGS)".to_string(),
        };
        let mut remote_flags: HashMap<u32, Flags> = HashMap::new();
        if sel.exists > 0 {
            // Stream: each response holds its read buffer, so collecting 100k of
            // them first would keep hundreds of MB alive.
            let mut stream = s.uid_fetch("1:*", &query).await?;
            while let Some(f) = stream.try_next().await? {
                let Some(uid) = f.uid else { continue };
                let (flags, deleted) = flags_of(&f);
                if !deleted {
                    remote_flags.insert(uid, flags);
                }
            }
            drop(stream);
        }
        // Full UID list for deletion detection.
        let all_uids: HashSet<u32> = if since.is_some() {
            if sel.exists == 0 { HashSet::new() } else { s.uid_search("ALL").await? }
        } else {
            remote_flags.keys().copied().collect()
        };

        for (uid, (id, _)) in local.iter() {
            if !all_uids.contains(uid) {
                ctx.store.remove_from_mailbox(id, &local_mb)?;
            }
        }
        for (uid, flags) in &remote_flags {
            if let Some((id, lf)) = local.get(uid)
                && lf != flags {
                    ctx.store.set_flags(id, flags)?;
                }
        }
        let mut new_uids: Vec<u32> = all_uids.iter().filter(|u| !local.contains_key(u)).copied().collect();
        new_uids.sort_unstable_by(|a, b| b.cmp(a));
        if !new_uids.is_empty() {
            let window = Utc::now() - chrono::Duration::days(ctx.settings.initial_sync_days as i64);
            let recent: HashSet<u32> = s.uid_search(format!("SINCE {}", imap_date(window))).await?.into_iter().collect();
            let (full_body, headers_only): (Vec<u32>, Vec<u32>) = new_uids.iter().partition(|u| recent.contains(u));
            let total = new_uids.len();
            let mut done = 0;
            for chunk in full_body.chunks(25) {
                ctx.progress(format!("{}: downloading {}/{}", mb.name, done, total));
                self.fetch_into(s, ctx, mb, uv, chunk, true).await?;
                done += chunk.len();
            }
            for chunk in headers_only.chunks(250) {
                ctx.progress(format!("{}: indexing {}/{}", mb.name, done, total));
                self.fetch_into(s, ctx, mb, uv, chunk, false).await?;
                done += chunk.len();
            }
        }
        // Optimistic placements the server never confirmed (e.g. a move whose copy we
        // could not re-identify) are superseded by the real copies fetched above.
        if !ctx.store.has_pending_ops(ctx.account_id())? {
            for id in ctx.store.mailbox_unconfirmed(&local_mb)? {
                ctx.store.remove_from_mailbox(&id, &local_mb)?;
            }
        }
        let c = MailboxCursor { uidvalidity: uv, uidnext: sel.uid_next.unwrap_or(0), modseq: sel.highest_modseq };
        ctx.set_cursor(&key, &serde_json::to_string(&c)?)?;
        Ok(())
    }

    pub(super) async fn fetch_into(&self, s: &mut Session, ctx: &SyncCtx, mb: &RemoteMailbox, uv: u32, uids: &[u32], body: bool) -> Result<()> {
        let q = if body { "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])" } else { "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])" };
        let mut stream = s.uid_fetch(uid_set(uids), q).await?;
        while let Some(f) = stream.try_next().await? {
            let Some(uid) = f.uid else { continue };
            let (flags, deleted) = flags_of(&f);
            if deleted {
                continue;
            }
            let rid = remote_id(uv, uid, &mb.remote_id);
            let internal = f.internal_date().map(|d| d.with_timezone(&Utc));
            let mbs = [(mb.remote_id.clone(), Some(uid))];
            if body
                && let Some(raw) = f.body() {
                    ctx.put_raw_message(&rid, None, raw, flags, &mbs, internal)?;
                    continue;
                }
            if let Some(hdr) = f.header().or(f.body()) {
                let p = satchel_mime::parse(hdr).unwrap_or_default();
                let rec = MessageRecord {
                    remote_id: rid,
                    message_id_hdr: p.message_id,
                    in_reply_to: p.in_reply_to,
                    references: p.references,
                    subject: p.subject,
                    from: p.from,
                    to: p.to,
                    cc: p.cc,
                    bcc: p.bcc,
                    reply_to: p.reply_to,
                    date: p.date.or(internal).unwrap_or_else(Utc::now),
                    flags,
                    size: f.size.unwrap_or(0) as u64,
                    ..Default::default()
                };
                ctx.put_record(rec, &mbs)?;
            }
        }
        Ok(())
    }
}
