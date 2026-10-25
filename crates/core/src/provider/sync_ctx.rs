//! [`SyncCtx`]: how providers write what they find, through the core.

use super::*;

/// Everything a provider needs to persist sync results.
#[derive(Clone)]
pub struct SyncCtx {
    pub account: Account,
    pub store: Arc<Store>,
    pub blobs: Arc<dyn BlobStore>,
    pub settings: Settings,
    pub(crate) events: tokio::sync::broadcast::Sender<CoreEvent>,
    /// New inbox mail stored since the last `messages_changed`.
    pub(crate) arrivals: Arc<std::sync::Mutex<Vec<NewMail>>>,
}

impl SyncCtx {
    pub fn account_id(&self) -> &str {
        &self.account.id
    }

    pub fn progress(&self, detail: impl Into<String>) {
        let st = SyncStatus { state: SyncState::Syncing, last_success: self.account.sync.last_success, error: None, detail: Some(detail.into()) };
        let _ = self.store.set_sync_status(&self.account.id, &st);
        let _ = self.events.send(CoreEvent::SyncStatus { account_id: self.account.id.clone(), status: st });
    }

    pub fn cursor(&self, key: &str) -> Result<Option<String>> {
        self.store.cursor(&self.account.id, key)
    }

    pub fn set_cursor(&self, key: &str, value: &str) -> Result<()> {
        self.store.set_cursor(&self.account.id, key, value)
    }

    /// Record the full mailbox list; prunes ones that disappeared. Returns
    /// (remote id -> local id).
    pub fn set_mailboxes(&self, mbs: &[RemoteMailbox]) -> Result<std::collections::HashMap<String, String>> {
        let mut map = std::collections::HashMap::new();
        for m in mbs {
            let id = self.store.upsert_mailbox(&self.account.id, &m.remote_id, &m.name, &m.path, m.role, m.is_label)?;
            map.insert(m.remote_id.clone(), id);
        }
        let keep: Vec<String> = mbs.iter().map(|m| m.remote_id.clone()).collect();
        self.store.prune_mailboxes(&self.account.id, &keep)?;
        let _ = self.events.send(CoreEvent::MailboxesChanged { account_id: self.account.id.clone() });
        Ok(map)
    }

    pub fn mailbox_local_id(&self, remote_id: &str) -> Result<Option<String>> {
        Ok(self.store.mailbox_by_remote(&self.account.id, remote_id)?.map(|m| m.id))
    }

    /// Store a message from raw RFC 822 bytes. `mailboxes` are remote mailbox ids with optional UIDs.
    pub fn put_raw_message(
        &self,
        remote_id: &str,
        remote_thread_id: Option<&str>,
        raw: &[u8],
        flags: Flags,
        mailboxes: &[(String, Option<u32>)],
        internal_date: Option<DateTime<Utc>>,
    ) -> Result<String> {
        let parsed = satchel_mime::parse(raw).unwrap_or_default();
        let hash = self.blobs.put(raw)?;
        let rec = MessageRecord {
            remote_id: remote_id.to_string(),
            remote_thread_id: remote_thread_id.map(|s| s.to_string()),
            message_id_hdr: parsed.message_id.clone(),
            in_reply_to: parsed.in_reply_to.clone(),
            references: parsed.references.clone(),
            subject: parsed.subject.clone(),
            from: parsed.from.clone(),
            to: parsed.to.clone(),
            cc: parsed.cc.clone(),
            bcc: parsed.bcc.clone(),
            reply_to: parsed.reply_to.clone(),
            date: parsed.date.or(internal_date).unwrap_or_else(Utc::now),
            snippet: parsed.snippet.clone(),
            flags,
            attachments: parsed.attachments.clone(),
            blob_hash: Some(hash),
            size: raw.len() as u64,
        };
        self.put_record(rec, mailboxes)
    }

    /// Store a message from metadata only (body fetched later).
    pub fn put_record(&self, rec: MessageRecord, mailboxes: &[(String, Option<u32>)]) -> Result<String> {
        let local: Vec<(String, Option<u32>)> = mailboxes
            .iter()
            .filter_map(|(r, uid)| self.mailbox_local_id(r).ok().flatten().map(|id| (id, *uid)))
            .collect();
        let (id, inserted) = self.store.upsert_message(&self.account.id, &rec, &local)?;
        if inserted && self.is_arrival(&rec, &local) {
            let thread_id = self.store.message_row(&id)?.map(|m| m.summary.thread_id).unwrap_or_default();
            self.arrivals.lock().unwrap().push(NewMail {
                message_id: id.clone(),
                thread_id,
                from: rec.from.clone(),
                subject: rec.subject.clone(),
                snippet: rec.snippet.clone(),
            });
        }
        Ok(id)
    }

    /// Whether a newly stored message deserves a notification: unread, in an
    /// inbox, recent, and not the first sync, a backfill, our own mail, or a
    /// sender the user marked as spam.
    fn is_arrival(&self, rec: &MessageRecord, local: &[(String, Option<u32>)]) -> bool {
        // No notifications until the account has synced once.
        let Some(last) = self.account.sync.last_success else { return false };
        if !self.settings.notify_new_mail || rec.flags.seen || rec.flags.draft || rec.date < last - chrono::Duration::days(1) {
            return false;
        }
        let from = rec.from.as_ref().map(|a| a.email.to_lowercase()).unwrap_or_default();
        if from == self.account.email.to_lowercase() || self.store.sender_rule(&from).ok().flatten() == Some(SenderVerdict::Spam) {
            return false;
        }
        local.iter().any(|(id, _)| self.store.get_mailbox(id).ok().flatten().is_some_and(|m| m.role == MailboxRole::Inbox))
    }

    pub fn message_by_remote(&self, remote_id: &str) -> Result<Option<String>> {
        self.store.message_id_by_remote(&self.account.id, remote_id)
    }

    pub fn set_flags_remote(&self, remote_id: &str, flags: Flags) -> Result<()> {
        if let Some(id) = self.message_by_remote(remote_id)? {
            self.store.set_flags(&id, &flags)?;
        }
        Ok(())
    }

    pub fn set_memberships_remote(&self, remote_id: &str, mailboxes: &[(String, Option<u32>)]) -> Result<()> {
        if let Some(id) = self.message_by_remote(remote_id)? {
            let local: Vec<(String, Option<u32>)> = mailboxes
                .iter()
                .filter_map(|(r, uid)| self.mailbox_local_id(r).ok().flatten().map(|mid| (mid, *uid)))
                .collect();
            if local.is_empty() {
                self.store.delete_message(&id)?;
            } else {
                self.store.set_memberships(&id, &local)?;
            }
        }
        Ok(())
    }

    pub fn delete_remote(&self, remote_id: &str) -> Result<()> {
        if let Some(id) = self.message_by_remote(remote_id)? {
            self.store.delete_message(&id)?;
        }
        Ok(())
    }

    pub fn messages_changed(&self) {
        let _ = self.store.recount_mailboxes(&self.account.id);
        let _ = self.events.send(CoreEvent::MessagesChanged { account_id: self.account.id.clone() });
        let messages = std::mem::take(&mut *self.arrivals.lock().unwrap());
        if !messages.is_empty() {
            let _ = self.events.send(CoreEvent::NewMail { account_id: self.account.id.clone(), messages });
        }
    }

    // calendars

    pub fn set_calendars(&self, cals: &[RemoteCalendar]) -> Result<std::collections::HashMap<String, String>> {
        let mut map = std::collections::HashMap::new();
        for c in cals {
            let id = self.store.upsert_calendar(&self.account.id, &c.remote_id, &c.name, c.color.as_deref(), c.read_only)?;
            map.insert(c.remote_id.clone(), id);
        }
        let keep: Vec<String> = cals.iter().map(|c| c.remote_id.clone()).collect();
        self.store.prune_calendars(&self.account.id, &keep)?;
        Ok(map)
    }

    /// Store an event's iCalendar object (all components for one UID/resource).
    pub fn put_event(&self, calendar_id: &str, remote_id: &str, etag: Option<&str>, ics: &str) -> Result<()> {
        let parsed = satchel_ical::parse(ics);
        let Some(master) = parsed.events.iter().find(|e| e.recurrence_id.is_none()).or(parsed.events.first()) else {
            return Ok(());
        };
        let (start, end) = crate::calendar::series_range(&parsed.events);
        self.store.upsert_event(
            calendar_id,
            &self.account.id,
            remote_id,
            etag,
            &master.uid,
            ics,
            &master.summary,
            (start, end),
            master.rrule.is_some(),
            false,
        )?;
        Ok(())
    }

    pub fn delete_event_remote(&self, calendar_id: &str, remote_id: &str) -> Result<()> {
        self.store.delete_event_by_remote(calendar_id, remote_id)
    }

    pub fn calendar_changed(&self) {
        let _ = self.events.send(CoreEvent::CalendarChanged { account_id: self.account.id.clone() });
    }

    // contacts

    /// Record the account's address books; drops ones that disappeared, and
    /// contacts that only lived in them. Returns (remote id -> local id).
    pub fn set_address_books(&self, books: &[RemoteAddressBook]) -> Result<std::collections::HashMap<String, String>> {
        let mut map = std::collections::HashMap::new();
        for b in books {
            map.insert(b.remote_id.clone(), self.store.upsert_address_book(&self.account.id, &b.remote_id, &b.name, b.read_only)?);
        }
        let keep: Vec<String> = books.iter().map(|b| b.remote_id.clone()).collect();
        self.store.prune_address_books(&self.account.id, &keep)?;
        self.store.prune_orphan_remote_contacts()?;
        Ok(map)
    }

    /// Store a card the server reported (new or changed). Server changes are
    /// merged field by field with local edits; local edits win.
    pub fn put_card(&self, address_book_id: &str, remote_id: &str, etag: Option<&str>, vcard: &str) -> Result<()> {
        crate::contacts::absorb_card(&self.store, &self.account.id, address_book_id, remote_id, etag, vcard)?;
        Ok(())
    }

    /// The server no longer has this card.
    pub fn delete_card_remote(&self, address_book_id: &str, remote_id: &str) -> Result<()> {
        crate::contacts::card_gone(&self.store, address_book_id, remote_id)
    }

    pub fn contacts_changed(&self) {
        let _ = self.events.send(CoreEvent::ContactsChanged);
    }
}
