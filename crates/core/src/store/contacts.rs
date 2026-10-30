//! Contacts, address books and cards.

use super::*;

impl Store {
    fn contact_row(r: &Row) -> rusqlite::Result<StoredContact> {
        Ok(StoredContact {
            id: r.get("id")?,
            fields: from_json(&r.get::<_, String>("fields_json")?),
            origin: parse_origin(&r.get::<_, String>("origin")?),
            deleted: r.get("deleted")?,
            version: r.get("version")?,
        })
    }

    fn write_contact_emails(c: &Connection, id: &str, fields: &ContactFields) -> Result<()> {
        c.execute("DELETE FROM contact_emails WHERE contact_id=?", [id])?;
        for e in &fields.emails {
            let email = e.email.trim().to_lowercase();
            if !email.is_empty() {
                c.execute("INSERT OR IGNORE INTO contact_emails (contact_id, email) VALUES (?,?)", params![id, email])?;
            }
        }
        Ok(())
    }

    pub fn insert_contact(&self, fields: &ContactFields, origin: ContactOrigin) -> Result<String> {
        let id = new_id();
        self.w(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO contacts (id, name, fields_json, origin, created_at, updated_at) VALUES (?,?,?,?,?,?)",
                params![id, fields.name.trim(), to_json(fields), origin_str(origin), now_ms(), now_ms()],
            )?;
            Self::write_contact_emails(&tx, &id, fields)?;
            tx.commit()?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Replace a contact's fields (and optionally its origin); bumps its version.
    pub fn update_contact(&self, id: &str, fields: &ContactFields, origin: Option<ContactOrigin>) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "UPDATE contacts SET name=?, fields_json=?, origin=COALESCE(?, origin), updated_at=?, version=version+1 WHERE id=?",
                params![fields.name.trim(), to_json(fields), origin.map(origin_str), now_ms(), id],
            )?;
            Self::write_contact_emails(&tx, id, fields)?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn get_contact(&self, id: &str) -> Result<Option<StoredContact>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM contacts WHERE id=?", [id], Self::contact_row).optional()?))
    }

    /// Live contacts sorted by name (address when there is none), optionally
    /// matching `search` in the name, an address or any other field.
    pub fn list_contacts(&self, search: Option<&str>, limit: u32) -> Result<Vec<StoredContact>> {
        self.r(|c| {
            let like = search.map(|s| format!("%{}%", s.trim().to_lowercase().replace(['%', '_'], "")));
            let mut s = c.prepare(
                "SELECT c.* FROM contacts c WHERE c.deleted=0
                   AND (?1 IS NULL OR lower(c.name) LIKE ?1 OR lower(c.fields_json) LIKE ?1
                        OR EXISTS (SELECT 1 FROM contact_emails e WHERE e.contact_id=c.id AND e.email LIKE ?1))
                 ORDER BY CASE WHEN c.name='' THEN (SELECT min(email) FROM contact_emails e WHERE e.contact_id=c.id) ELSE lower(c.name) END, c.id
                 LIMIT ?2",
            )?;
            let rows = s.query_map(params![like, limit], Self::contact_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Contacts (tombstones included) that have this address.
    pub fn contacts_by_email(&self, email: &str) -> Result<Vec<StoredContact>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT c.* FROM contacts c JOIN contact_emails e ON e.contact_id=c.id WHERE e.email=? ORDER BY c.created_at")?;
            let rows = s.query_map([email.trim().to_lowercase()], Self::contact_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Every address that belongs to some contact, deleted ones included.
    pub fn claimed_emails(&self) -> Result<HashSet<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT email FROM contact_emails")?;
            let rows = s.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<HashSet<String>>>()?;
            Ok(rows)
        })
    }

    /// Live automatic contacts, for refreshing their names.
    pub fn auto_contacts(&self) -> Result<Vec<StoredContact>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM contacts WHERE origin='auto' AND deleted=0")?;
            let rows = s.query_map([], Self::contact_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Hide a contact for good: it keeps its addresses so automatic generation
    /// skips them, and loses its server copies.
    pub fn tombstone_contact(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE contacts SET deleted=1, version=version+1, updated_at=? WHERE id=?", params![now_ms(), id])?;
            c.execute("DELETE FROM contact_cards WHERE contact_id=?", [id])?;
            Ok(())
        })
    }

    pub fn delete_contact(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM contacts WHERE id=?", [id])?;
            Ok(())
        })
    }

    /// Contacts that only existed in an address book that is gone (account
    /// removed, book deleted on the server).
    pub fn prune_orphan_remote_contacts(&self) -> Result<usize> {
        self.w(|c| {
            Ok(c.execute(
                "DELETE FROM contacts WHERE origin='remote' AND deleted=0 AND NOT EXISTS (SELECT 1 FROM contact_cards k WHERE k.contact_id=contacts.id)",
                [],
            )?)
        })
    }

    /// Address books each contact is saved in.
    pub fn contact_sources(&self) -> Result<HashMap<String, Vec<ContactSource>>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT k.contact_id, b.id, b.account_id, b.name FROM contact_cards k JOIN address_books b ON b.id=k.address_book_id ORDER BY b.name",
            )?;
            let mut out: HashMap<String, Vec<ContactSource>> = HashMap::new();
            for row in s.query_map([], |r| Ok((r.get::<_, String>(0)?, ContactSource { address_book_id: r.get(1)?, account_id: r.get(2)?, name: r.get(3)? })))? {
                let (cid, src) = row?;
                out.entry(cid).or_default().push(src);
            }
            Ok(out)
        })
    }

    // -- correspondence (derived from mail) ---------------------------------------

    /// Raw evidence of correspondence: (address, display name, date ms, named by
    /// themselves). Recipients of mail the user sent, plus senders the user
    /// answered or wrote to in the same thread. Spam-folder mail doesn't count.
    /// Sent mail is mail in a Sent folder: anyone can put your address in From.
    pub fn correspondence_evidence(&self) -> Result<Vec<Evidence>> {
        self.r(|c| {
            let mut s = c.prepare(
                "WITH own AS (SELECT lower(email) AS e FROM accounts),
                 sent AS (SELECT m.thread_id, m.to_json, m.cc_json, m.bcc_json, m.date FROM messages m
                          WHERE m.draft=0 AND lower(m.from_email) IN (SELECT e FROM own)
                            AND EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id
                                        WHERE mm.message_id=m.id AND mb.role='sent'))
                 SELECT lower(json_extract(j.value, '$.email')), json_extract(j.value, '$.name'), s.date, 0 FROM sent s, json_each(s.to_json) j
                 UNION ALL
                 SELECT lower(json_extract(j.value, '$.email')), json_extract(j.value, '$.name'), s.date, 0 FROM sent s, json_each(s.cc_json) j
                 UNION ALL
                 SELECT lower(json_extract(j.value, '$.email')), json_extract(j.value, '$.name'), s.date, 0 FROM sent s, json_each(s.bcc_json) j
                 UNION ALL
                 SELECT lower(m.from_email), m.from_name, m.date, 1 FROM messages m
                 WHERE m.from_email IS NOT NULL AND m.draft=0 AND lower(m.from_email) NOT IN (SELECT e FROM own)
                   AND (m.answered=1 OR m.thread_id IN (SELECT thread_id FROM sent))
                   AND NOT EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id
                                   WHERE mm.message_id=m.id AND mb.role='junk')",
            )?;
            let rows = s
                .query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?)))?
                .filter_map(|r| match r {
                    Ok((Some(e), n, d, own)) => Some(Ok((e, n, d, own))),
                    Ok((None, ..)) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// (messages, last date) per lowercase address: mail from it plus mail you
    /// sent to it, drafts excluded. The same counts as `person_summary`.
    pub fn address_stats(&self) -> Result<HashMap<String, (u32, DateTime<Utc>)>> {
        self.r(|c| {
            let mut s = c.prepare(&format!(
                "SELECT e, SUM(n), MAX(d) FROM (
                   SELECT lower(from_email) e, COUNT(*) n, MAX(date) d FROM messages WHERE draft=0 AND from_email IS NOT NULL GROUP BY 1
                   UNION ALL
                   SELECT e, COUNT(DISTINCT id), MAX(date) FROM ({SENT_TO}) GROUP BY e
                 ) GROUP BY e"
            ))?;
            let rows = s
                .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, u32>(1)?, ms_to_dt(r.get(2)?)))))?
                .collect::<rusqlite::Result<HashMap<_, _>>>()?;
            Ok(rows)
        })
    }

    /// [`Self::address_stats`] counting only messages dated `after` or later.
    pub fn address_stats_since(&self, after: DateTime<Utc>) -> Result<HashMap<String, (u32, DateTime<Utc>)>> {
        self.r(|c| {
            let mut s = c.prepare(&format!(
                "SELECT e, SUM(n), MAX(d) FROM (
                   SELECT lower(from_email) e, COUNT(*) n, MAX(date) d FROM messages WHERE draft=0 AND from_email IS NOT NULL AND date>=?1 GROUP BY 1
                   UNION ALL
                   SELECT e, COUNT(DISTINCT id), MAX(date) FROM ({SENT_TO}) WHERE date>=?1 GROUP BY e
                 ) GROUP BY e"
            ))?;
            let rows = s
                .query_map([after.timestamp_millis()], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, u32>(1)?, ms_to_dt(r.get(2)?)))))?
                .collect::<rusqlite::Result<HashMap<_, _>>>()?;
            Ok(rows)
        })
    }

    pub fn contact_emails_of(&self, id: &str) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT email FROM contact_emails WHERE contact_id=?")?;
            let rows = s.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    // -- address books & cards ---------------------------------------------------

    pub fn upsert_address_book(&self, account_id: &str, remote_id: &str, name: &str, read_only: bool) -> Result<String> {
        self.w(|c| {
            let existing: Option<String> =
                c.query_row("SELECT id FROM address_books WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0)).optional()?;
            if let Some(id) = existing {
                c.execute("UPDATE address_books SET name=?, read_only=? WHERE id=?", params![name, read_only, id])?;
                return Ok(id);
            }
            let id = new_id();
            c.execute(
                "INSERT INTO address_books (id, account_id, remote_id, name, read_only) VALUES (?,?,?,?,?)",
                params![id, account_id, remote_id, name, read_only],
            )?;
            Ok(id)
        })
    }

    fn address_book_row(r: &Row) -> rusqlite::Result<StoredAddressBook> {
        Ok(StoredAddressBook {
            book: AddressBook { id: r.get("id")?, account_id: r.get("account_id")?, name: r.get("name")?, read_only: r.get("read_only")? },
            remote_id: r.get("remote_id")?,
            sync_token: r.get("sync_token")?,
        })
    }

    pub fn list_address_books(&self, account_id: Option<&str>) -> Result<Vec<StoredAddressBook>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM address_books WHERE ?1 IS NULL OR account_id=?1 ORDER BY account_id, name")?;
            let rows = s.query_map([account_id], Self::address_book_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_address_book(&self, id: &str) -> Result<Option<StoredAddressBook>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM address_books WHERE id=?", [id], Self::address_book_row).optional()?))
    }

    pub fn set_address_book_token(&self, id: &str, token: Option<&str>) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE address_books SET sync_token=? WHERE id=?", params![token, id])?;
            Ok(())
        })
    }

    /// Drop address books the account no longer has (their cards go with them).
    pub fn prune_address_books(&self, account_id: &str, keep_remote: &[String]) -> Result<()> {
        let all = self.list_address_books(Some(account_id))?;
        self.w(|c| {
            for b in all {
                if !keep_remote.contains(&b.remote_id) {
                    c.execute("DELETE FROM address_books WHERE id=?", [&b.book.id])?;
                }
            }
            Ok(())
        })
    }

    fn card_row(r: &Row) -> rusqlite::Result<StoredCard> {
        Ok(StoredCard {
            id: r.get("id")?,
            contact_id: r.get("contact_id")?,
            address_book_id: r.get("address_book_id")?,
            account_id: r.get("account_id")?,
            remote_id: r.get("remote_id")?,
            etag: r.get("etag")?,
            uid: r.get("uid")?,
            vcard: r.get("vcard")?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_card(&self, contact_id: &str, address_book_id: &str, account_id: &str, remote_id: &str, etag: Option<&str>, uid: &str, vcard: &str) -> Result<String> {
        let id = new_id();
        self.w(|c| {
            c.execute(
                "INSERT INTO contact_cards (id, contact_id, address_book_id, account_id, remote_id, etag, uid, vcard) VALUES (?,?,?,?,?,?,?,?)",
                params![id, contact_id, address_book_id, account_id, remote_id, etag, uid, vcard],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    pub fn get_card(&self, id: &str) -> Result<Option<StoredCard>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM contact_cards WHERE id=?", [id], Self::card_row).optional()?))
    }

    pub fn card_by_remote(&self, address_book_id: &str, remote_id: &str) -> Result<Option<StoredCard>> {
        self.r(|c| {
            Ok(c.query_row("SELECT * FROM contact_cards WHERE address_book_id=? AND remote_id=?", [address_book_id, remote_id], Self::card_row)
                .optional()?)
        })
    }

    /// A card created here and not yet confirmed by the server, by vCard UID.
    pub fn pending_card_by_uid(&self, address_book_id: &str, uid: &str) -> Result<Option<StoredCard>> {
        self.r(|c| {
            Ok(c.query_row(
                "SELECT * FROM contact_cards WHERE address_book_id=? AND uid=? AND remote_id LIKE 'local:%'",
                [address_book_id, uid],
                Self::card_row,
            )
            .optional()?)
        })
    }

    pub fn cards_of_contact(&self, contact_id: &str) -> Result<Vec<StoredCard>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM contact_cards WHERE contact_id=?")?;
            let rows = s.query_map([contact_id], Self::card_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// remote id -> (card id, etag) for cards the server has confirmed.
    pub fn card_remote_ids(&self, address_book_id: &str) -> Result<HashMap<String, (String, Option<String>)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT remote_id, id, etag FROM contact_cards WHERE address_book_id=? AND remote_id NOT LIKE 'local:%'")?;
            let rows = s.query_map([address_book_id], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
            Ok(rows)
        })
    }

    /// Record what the server now holds for a card.
    pub fn set_card_remote(&self, id: &str, remote_id: &str, etag: Option<&str>, vcard: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE contact_cards SET remote_id=?, etag=?, vcard=? WHERE id=?", params![remote_id, etag, vcard, id])?;
            Ok(())
        })
    }

    pub fn delete_card(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM contact_cards WHERE id=?", [id])?;
            Ok(())
        })
    }

    /// Move every card of `from` to `to` (merging contacts).
    pub fn relink_cards(&self, from: &str, to: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE contact_cards SET contact_id=? WHERE contact_id=?", params![to, from])?;
            Ok(())
        })
    }
}
