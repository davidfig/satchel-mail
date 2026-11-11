//! Cheap fingerprints of what each part of the AI folder is built from, so
//! a pass can skip a part when nothing it shows has changed.

use super::*;

/// What [`Store::stamps`] returns: one value per part of the AI folder.
/// Equal stamps mean nothing that part reads changed (row counts plus
/// version sums: every edit bumps a row's version). Not stable across
/// versions of Satchel; only compared within one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamps {
    /// Mail indexes and history: messages, threads, mailboxes, written
    /// thread folders, accounts, favorite senders.
    pub mail: u64,
    /// Events, calendars, accounts.
    pub calendar: u64,
    /// Contacts and the address books they're in, accounts.
    pub contacts: u64,
}

impl Store {
    pub fn stamps(&self) -> Result<Stamps> {
        use std::hash::{Hash, Hasher};
        self.r(|c| {
            let q = |sql: &str| -> Result<String> { Ok(c.query_row(sql, [], |r| r.get::<_, String>(0))?) };
            let hash = |parts: &[&String]| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                parts.hash(&mut h);
                h.finish()
            };
            let accounts = q("SELECT count(*) || ':' || ifnull(group_concat(id || '=' || email || '/' || send_as_json), '') FROM accounts")?;
            let mail = [
                q("SELECT count(*) || ':' || total(version) FROM messages")?,
                q("SELECT count(*) || ':' || total(message_count) || ':' || total(last_date) FROM threads")?,
                q("SELECT count(*) || ':' || ifnull(group_concat(id || '=' || path || '/' || name || '/' || ifnull(role, '')), '') FROM mailboxes")?,
                q("SELECT count(*) || ':' || total(length(path)) FROM mirror_paths")?,
                q("SELECT count(*) || ':' || ifnull(group_concat(email), '') FROM favorite_senders")?,
            ];
            let calendar = [
                q("SELECT count(*) || ':' || total(version) FROM events")?,
                q("SELECT count(*) || ':' || ifnull(group_concat(id || '=' || name || '/' || account_id), '') FROM calendars")?,
            ];
            let contacts = [
                q("SELECT count(*) || ':' || total(version) FROM contacts")?,
                q("SELECT count(*) || ':' || ifnull(group_concat(id || '=' || name), '') FROM address_books")?,
                q("SELECT count(*) || ':' || ifnull(group_concat(contact_id || '=' || address_book_id), '') FROM contact_cards")?,
            ];
            Ok(Stamps {
                mail: hash(&[&accounts, &mail[0], &mail[1], &mail[2], &mail[3], &mail[4]]),
                calendar: hash(&[&accounts, &calendar[0], &calendar[1]]),
                contacts: hash(&[&accounts, &contacts[0], &contacts[1], &contacts[2]]),
            })
        })
    }
}
