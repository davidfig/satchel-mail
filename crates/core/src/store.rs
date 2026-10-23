//! SQLite-backed store. All SQL lives in this module so a different backend
//! (Postgres for the hosted version) can be swapped in behind the same API.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use satchel_api::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn ms_to_dt(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_else(Utc::now)
}

pub fn new_id() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}

fn to_json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

fn from_json<T: DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

pub fn role_str(r: MailboxRole) -> &'static str {
    match r {
        MailboxRole::Inbox => "inbox",
        MailboxRole::Sent => "sent",
        MailboxRole::Drafts => "drafts",
        MailboxRole::Trash => "trash",
        MailboxRole::Junk => "junk",
        MailboxRole::Archive => "archive",
        MailboxRole::All => "all",
        MailboxRole::Flagged => "flagged",
        MailboxRole::Important => "important",
        MailboxRole::Custom => "custom",
    }
}

pub fn parse_role(s: &str) -> MailboxRole {
    match s {
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

fn kind_str(k: ProviderKind) -> &'static str {
    match k {
        ProviderKind::Imap => "imap",
        ProviderKind::Gmail => "gmail",
        ProviderKind::Graph => "graph",
        ProviderKind::Jmap => "jmap",
    }
}

fn sync_state_str(s: SyncState) -> &'static str {
    match s {
        SyncState::Idle => "idle",
        SyncState::Syncing => "syncing",
        SyncState::Error => "error",
        SyncState::Offline => "offline",
    }
}

fn parse_sync_state(s: &str) -> SyncState {
    match s {
        "syncing" => SyncState::Syncing,
        "error" => SyncState::Error,
        "offline" => SyncState::Offline,
        _ => SyncState::Idle,
    }
}

pub fn proposal_status_str(s: ProposalStatus) -> &'static str {
    match s {
        ProposalStatus::Pending => "pending",
        ProposalStatus::Approved => "approved",
        ProposalStatus::Applied => "applied",
        ProposalStatus::Rejected => "rejected",
        ProposalStatus::Stale => "stale",
        ProposalStatus::Failed => "failed",
    }
}

fn parse_proposal_status(s: &str) -> ProposalStatus {
    match s {
        "approved" => ProposalStatus::Approved,
        "applied" => ProposalStatus::Applied,
        "rejected" => ProposalStatus::Rejected,
        "stale" => ProposalStatus::Stale,
        "failed" => ProposalStatus::Failed,
        _ => ProposalStatus::Pending,
    }
}

const PALETTE: &[&str] = &["#4f7cff", "#e5484d", "#30a46c", "#f76b15", "#8e4ec6", "#12a594", "#d6409f", "#978365"];

/// A message as written by sync.
#[derive(Debug, Clone, Default)]
pub struct MessageRecord {
    pub remote_id: String,
    pub remote_thread_id: Option<String>,
    pub message_id_hdr: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub date: DateTime<Utc>,
    pub snippet: String,
    pub flags: Flags,
    pub attachments: Vec<Attachment>,
    pub blob_hash: Option<String>,
    pub size: u64,
}

/// Remote handles for a local message, used when pushing ops.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct MessageRef {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub remote_thread_id: Option<String>,
    pub message_id_hdr: Option<String>,
    /// (mailbox local id, mailbox remote id, uid)
    pub mailboxes: Vec<(String, String, Option<u32>)>,
    pub flags: Flags,
    pub blob_hash: Option<String>,
    pub version: i64,
}

#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: String,
    pub calendar_id: String,
    pub account_id: String,
    pub remote_id: String,
    pub etag: Option<String>,
    pub uid: String,
    pub ics: String,
    pub version: i64,
    pub pending: bool,
}

#[derive(Debug, Clone)]
pub struct StoredCalendar {
    pub cal: Calendar,
    pub remote_id: String,
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OpRow {
    pub id: String,
    pub account_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub attempts: u32,
    pub actor: String,
}

pub struct Store {
    write: Mutex<Connection>,
    read: Mutex<Connection>,
}

fn open_conn(path: &Path) -> Result<Connection> {
    let c = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    c.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;",
    )?;
    Ok(c)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let w = open_conn(path)?;
        w.execute_batch(include_str!("schema.sql"))?;
        let r = open_conn(path)?;
        Ok(Self { write: Mutex::new(w), read: Mutex::new(r) })
    }

    pub fn open_in_memory() -> Result<Self> {
        // Shared-cache in-memory DB so both connections see the same data.
        let name = format!("file:mem{}?mode=memory&cache=shared", new_id());
        let w = Connection::open_with_flags(
            &name,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        w.execute_batch("PRAGMA foreign_keys=ON;")?;
        w.execute_batch(include_str!("schema.sql"))?;
        let r = Connection::open_with_flags(
            &name,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        Ok(Self { write: Mutex::new(w), read: Mutex::new(r) })
    }

    fn w<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut c = self.write.lock().map_err(|_| anyhow!("store poisoned"))?;
        f(&mut c)
    }

    fn r<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let c = self.read.lock().map_err(|_| anyhow!("store poisoned"))?;
        f(&c)
    }

    /// Changes whenever another connection (or process) commits.
    pub fn data_version(&self) -> Result<i64> {
        self.r(|c| Ok(c.query_row("PRAGMA data_version", [], |r| r.get(0))?))
    }

    // -----------------------------------------------------------------------
    // Accounts
    // -----------------------------------------------------------------------

    pub fn insert_account(&self, config: &AccountConfig) -> Result<Account> {
        let id = new_id();
        self.w(|c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
            let color = PALETTE[n as usize % PALETTE.len()];
            c.execute(
                "INSERT INTO accounts (id, kind, email, display_name, config_json, color, created_at) VALUES (?,?,?,?,?,?,?)",
                params![id, kind_str(config.kind()), config.email(), config.display_name(), to_json(config), color, now_ms()],
            )?;
            Ok(())
        })?;
        self.get_account(&id)?.ok_or_else(|| anyhow!("account vanished"))
    }

    fn account_row(r: &Row) -> rusqlite::Result<Account> {
        let config: String = r.get("config_json")?;
        let config: AccountConfig = serde_json::from_str(&config).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(Account {
            id: r.get("id")?,
            kind: config.kind(),
            email: r.get("email")?,
            display_name: r.get("display_name")?,
            config,
            color: r.get("color")?,
            sync: SyncStatus {
                state: parse_sync_state(&r.get::<_, String>("sync_state")?),
                last_success: r.get::<_, Option<i64>>("last_success")?.map(ms_to_dt),
                error: r.get("error")?,
                detail: r.get("detail")?,
            },
        })
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM accounts ORDER BY created_at")?;
            let rows = s.query_map([], Self::account_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM accounts WHERE id = ?", [id], Self::account_row).optional()?))
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM accounts WHERE id = ?", [id])?;
            c.execute("DELETE FROM threads WHERE account_id = ?", [id])?;
            Ok(())
        })
    }

    pub fn set_sync_status(&self, id: &str, st: &SyncStatus) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE accounts SET sync_state=?, last_success=?, error=?, detail=? WHERE id=?",
                params![sync_state_str(st.state), st.last_success.map(|d| d.timestamp_millis()), st.error, st.detail, id],
            )?;
            Ok(())
        })
    }

    pub fn update_account_config(&self, id: &str, config: &AccountConfig) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE accounts SET config_json=? WHERE id=?", params![to_json(config), id])?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Sync cursors
    // -----------------------------------------------------------------------

    pub fn cursor(&self, account_id: &str, key: &str) -> Result<Option<String>> {
        self.r(|c| {
            Ok(c.query_row("SELECT value FROM sync_cursors WHERE account_id=? AND key=?", [account_id, key], |r| r.get(0))
                .optional()?)
        })
    }

    pub fn set_cursor(&self, account_id: &str, key: &str, value: &str) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO sync_cursors (account_id, key, value) VALUES (?,?,?) ON CONFLICT(account_id, key) DO UPDATE SET value=excluded.value",
                params![account_id, key, value],
            )?;
            Ok(())
        })
    }

    pub fn delete_cursor(&self, account_id: &str, key: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM sync_cursors WHERE account_id=? AND key=?", [account_id, key])?;
            Ok(())
        })
    }

    pub fn delete_cursors_with_prefix(&self, account_id: &str, prefix: &str) -> Result<()> {
        self.w(|c| {
            c.execute(
                "DELETE FROM sync_cursors WHERE account_id=? AND key LIKE ?",
                params![account_id, format!("{prefix}%")],
            )?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Mailboxes
    // -----------------------------------------------------------------------

    pub fn upsert_mailbox(&self, account_id: &str, remote_id: &str, name: &str, path: &str, role: MailboxRole, is_label: bool) -> Result<String> {
        self.w(|c| {
            let existing: Option<String> = c
                .query_row("SELECT id FROM mailboxes WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?;
            if let Some(id) = existing {
                c.execute(
                    "UPDATE mailboxes SET name=?, path=?, role=?, is_label=? WHERE id=?",
                    params![name, path, role_str(role), is_label, id],
                )?;
                return Ok(id);
            }
            let id = new_id();
            c.execute(
                "INSERT INTO mailboxes (id, account_id, remote_id, name, path, role, is_label) VALUES (?,?,?,?,?,?,?)",
                params![id, account_id, remote_id, name, path, role_str(role), is_label],
            )?;
            Ok(id)
        })
    }

    fn mailbox_row(r: &Row) -> rusqlite::Result<Mailbox> {
        Ok(Mailbox {
            id: r.get("id")?,
            account_id: r.get("account_id")?,
            name: r.get("name")?,
            path: r.get("path")?,
            role: parse_role(&r.get::<_, String>("role")?),
            is_label: r.get("is_label")?,
            unread: r.get("unread")?,
            total: r.get("total")?,
        })
    }

    pub fn list_mailboxes(&self, account_id: Option<&str>) -> Result<Vec<Mailbox>> {
        self.r(|c| {
            let order = "ORDER BY account_id, CASE role WHEN 'inbox' THEN 0 WHEN 'flagged' THEN 1 WHEN 'drafts' THEN 2 WHEN 'sent' THEN 3 WHEN 'archive' THEN 4 WHEN 'all' THEN 5 WHEN 'junk' THEN 6 WHEN 'trash' THEN 7 ELSE 8 END, path";
            let rows = match account_id {
                Some(a) => {
                    let mut s = c.prepare(&format!("SELECT * FROM mailboxes WHERE account_id=? {order}"))?;
                    s.query_map([a], Self::mailbox_row)?.collect::<rusqlite::Result<Vec<_>>>()?
                }
                None => {
                    let mut s = c.prepare(&format!("SELECT * FROM mailboxes {order}"))?;
                    s.query_map([], Self::mailbox_row)?.collect::<rusqlite::Result<Vec<_>>>()?
                }
            };
            Ok(rows)
        })
    }

    pub fn get_mailbox(&self, id: &str) -> Result<Option<Mailbox>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM mailboxes WHERE id=?", [id], Self::mailbox_row).optional()?))
    }

    pub fn mailbox_remote_id(&self, id: &str) -> Result<Option<String>> {
        self.r(|c| Ok(c.query_row("SELECT remote_id FROM mailboxes WHERE id=?", [id], |r| r.get(0)).optional()?))
    }

    pub fn mailbox_by_remote(&self, account_id: &str, remote_id: &str) -> Result<Option<Mailbox>> {
        self.r(|c| {
            Ok(c.query_row("SELECT * FROM mailboxes WHERE account_id=? AND remote_id=?", [account_id, remote_id], Self::mailbox_row)
                .optional()?)
        })
    }

    pub fn mailbox_by_role(&self, account_id: &str, role: MailboxRole) -> Result<Option<Mailbox>> {
        self.r(|c| {
            Ok(c.query_row(
                "SELECT * FROM mailboxes WHERE account_id=? AND role=? ORDER BY length(path) LIMIT 1",
                [account_id, role_str(role)],
                Self::mailbox_row,
            )
            .optional()?)
        })
    }

    /// Remove mailboxes not in `keep_remote_ids`.
    pub fn prune_mailboxes(&self, account_id: &str, keep_remote_ids: &[String]) -> Result<()> {
        let keep: HashSet<&String> = keep_remote_ids.iter().collect();
        let all = self.list_mailboxes(Some(account_id))?;
        let ids: Vec<String> = all
            .into_iter()
            .filter_map(|m| {
                let rid = self.mailbox_remote_id(&m.id).ok().flatten()?;
                (!keep.contains(&rid)).then_some(m.id)
            })
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let affected = self.w(|c| {
            let tx = c.transaction()?;
            let mut threads = HashSet::new();
            for id in &ids {
                let mut s = tx.prepare("SELECT m.thread_id FROM messages m JOIN message_mailboxes mm ON mm.message_id=m.id WHERE mm.mailbox_id=?")?;
                for t in s.query_map([id], |r| r.get::<_, String>(0))? {
                    threads.insert(t?);
                }
                drop(s);
                tx.execute("DELETE FROM mailboxes WHERE id=?", [id])?;
            }
            // Messages now without any mailbox are gone from the server.
            tx.execute(
                "DELETE FROM messages WHERE account_id=? AND NOT EXISTS (SELECT 1 FROM message_mailboxes mm WHERE mm.message_id=messages.id)",
                [account_id],
            )?;
            tx.commit()?;
            Ok(threads)
        })?;
        for t in affected {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    pub fn recount_mailboxes(&self, account_id: &str) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE mailboxes SET
                   total = (SELECT COUNT(*) FROM message_mailboxes mm WHERE mm.mailbox_id = mailboxes.id),
                   unread = (SELECT COUNT(*) FROM message_mailboxes mm JOIN messages m ON m.id = mm.message_id WHERE mm.mailbox_id = mailboxes.id AND m.seen = 0)
                 WHERE account_id = ?",
                [account_id],
            )?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Messages
    // -----------------------------------------------------------------------

    pub fn message_id_by_remote(&self, account_id: &str, remote_id: &str) -> Result<Option<String>> {
        self.r(|c| {
            Ok(c.query_row("SELECT id FROM messages WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?)
        })
    }

    /// Insert or update a message and its mailbox memberships. Returns
    /// (local id, inserted?).
    pub fn upsert_message(&self, account_id: &str, rec: &MessageRecord, mailboxes: &[(String, Option<u32>)]) -> Result<(String, bool)> {
        let (id, inserted, threads) = self.w(|c| {
            let tx = c.transaction()?;
            let existing: Option<(String, String)> = tx
                .query_row("SELECT id, thread_id FROM messages WHERE account_id=? AND remote_id=?", [account_id, &rec.remote_id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            let mut affected_threads: Vec<String> = vec![];
            let (id, inserted) = if let Some((id, thread_id)) = existing {
                tx.execute(
                    "UPDATE messages SET seen=?, flagged=?, answered=?, draft=?, remote_thread_id=COALESCE(?, remote_thread_id),
                       blob_hash=COALESCE(?, blob_hash), snippet=CASE WHEN ?='' THEN snippet ELSE ? END,
                       attachments_json=CASE WHEN ?='[]' THEN attachments_json ELSE ? END,
                       has_attachments=MAX(has_attachments, ?), version=version+1, mirrored=0
                     WHERE id=?",
                    params![
                        rec.flags.seen, rec.flags.flagged, rec.flags.answered, rec.flags.draft, rec.remote_thread_id,
                        rec.blob_hash, rec.snippet, rec.snippet, to_json(&rec.attachments), to_json(&rec.attachments),
                        rec.attachments.iter().any(|a| !a.inline), id
                    ],
                )?;
                affected_threads.push(thread_id);
                (id, false)
            } else {
                let id = new_id();
                let (thread_id, merge) = Self::assign_thread(&tx, account_id, rec)?;
                tx.execute(
                    "INSERT INTO messages (id, account_id, remote_id, remote_thread_id, thread_id, message_id_hdr, in_reply_to, subject,
                       from_name, from_email, to_json, cc_json, bcc_json, reply_to_json, refs_json, date, snippet, seen, flagged, answered,
                       draft, has_attachments, attachments_json, blob_hash, size)
                     VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        id, account_id, rec.remote_id, rec.remote_thread_id, thread_id, rec.message_id_hdr, rec.in_reply_to, rec.subject,
                        rec.from.as_ref().and_then(|a| a.name.clone()), rec.from.as_ref().map(|a| a.email.clone()),
                        to_json(&rec.to), to_json(&rec.cc), to_json(&rec.bcc), to_json(&rec.reply_to), to_json(&rec.references),
                        rec.date.timestamp_millis(), rec.snippet, rec.flags.seen, rec.flags.flagged, rec.flags.answered, rec.flags.draft,
                        rec.attachments.iter().any(|a| !a.inline), to_json(&rec.attachments), rec.blob_hash, rec.size as i64
                    ],
                )?;
                let mut refs: Vec<&String> = rec.references.iter().collect();
                if let Some(irt) = &rec.in_reply_to {
                    refs.push(irt);
                }
                for r in refs.into_iter().collect::<HashSet<_>>() {
                    tx.execute("INSERT INTO message_refs (message_id, account_id, ref) VALUES (?,?,?)", params![id, account_id, r])?;
                }
                for other in &merge {
                    tx.execute("UPDATE messages SET thread_id=? WHERE thread_id=?", params![thread_id, other])?;
                    tx.execute("DELETE FROM threads WHERE id=?", [other])?;
                }
                affected_threads.push(thread_id);
                (id, true)
            };
            Self::set_memberships_tx(&tx, &id, mailboxes)?;
            tx.commit()?;
            Ok((id, inserted, affected_threads))
        })?;
        for t in threads {
            self.recompute_thread(&t)?;
        }
        Ok((id, inserted))
    }

    /// Pick a thread for a new message. Returns (thread id, other threads to merge into it).
    fn assign_thread(tx: &rusqlite::Transaction, account_id: &str, rec: &MessageRecord) -> Result<(String, Vec<String>)> {
        if let Some(rt) = &rec.remote_thread_id {
            let t: Option<String> = tx
                .query_row("SELECT thread_id FROM messages WHERE account_id=? AND remote_thread_id=? LIMIT 1", [account_id, rt], |r| r.get(0))
                .optional()?;
            return Ok((t.unwrap_or_else(new_id), vec![]));
        }
        let mut found: Vec<String> = vec![];
        let mut ids: Vec<String> = rec.references.clone();
        if let Some(irt) = &rec.in_reply_to {
            ids.push(irt.clone());
        }
        if let Some(mid) = &rec.message_id_hdr {
            ids.push(mid.clone());
        }
        if !ids.is_empty() {
            let placeholders = vec!["?"; ids.len()].join(",");
            let mut s = tx.prepare(&format!(
                "SELECT DISTINCT thread_id FROM messages WHERE account_id=? AND message_id_hdr IN ({placeholders})"
            ))?;
            let mut p: Vec<String> = vec![account_id.to_string()];
            p.extend(ids.iter().cloned());
            for t in s.query_map(params_from_iter(p.iter()), |r| r.get::<_, String>(0))? {
                found.push(t?);
            }
        }
        if let Some(mid) = &rec.message_id_hdr {
            // Replies that arrived before their parent.
            let mut s = tx.prepare(
                "SELECT DISTINCT m.thread_id FROM message_refs r JOIN messages m ON m.id=r.message_id WHERE r.account_id=? AND r.ref=?",
            )?;
            for t in s.query_map([account_id, mid], |r| r.get::<_, String>(0))? {
                let t = t?;
                if !found.contains(&t) {
                    found.push(t);
                }
            }
        }
        found.dedup();
        match found.len() {
            0 => Ok((new_id(), vec![])),
            _ => {
                let first = found.remove(0);
                Ok((first, found))
            }
        }
    }

    fn set_memberships_tx(tx: &rusqlite::Transaction, message_id: &str, mailboxes: &[(String, Option<u32>)]) -> Result<()> {
        tx.execute("DELETE FROM message_mailboxes WHERE message_id=?", [message_id])?;
        for (mb, uid) in mailboxes {
            tx.execute(
                "INSERT OR REPLACE INTO message_mailboxes (message_id, mailbox_id, uid) VALUES (?,?,?)",
                params![message_id, mb, uid],
            )?;
        }
        Ok(())
    }

    /// Replace the remote identity of a message (e.g. IMAP MOVE assigned a new UID).
    pub fn rekey_message(&self, message_id: &str, new_remote_id: &str, mailboxes: &[(String, Option<u32>)]) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            // If sync already inserted the new copy, drop it in favour of the existing row.
            let dup: Option<String> = tx
                .query_row(
                    "SELECT id FROM messages WHERE remote_id=? AND account_id=(SELECT account_id FROM messages WHERE id=?) AND id<>?",
                    params![new_remote_id, message_id, message_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(d) = dup {
                tx.execute("DELETE FROM messages WHERE id=?", [d])?;
            }
            tx.execute("UPDATE messages SET remote_id=?, version=version+1, mirrored=0 WHERE id=?", params![new_remote_id, message_id])?;
            Self::set_memberships_tx(&tx, message_id, mailboxes)?;
            tx.commit()?;
            Ok(())
        })?;
        if let Some(t) = self.thread_of(message_id)? {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    pub fn set_memberships(&self, message_id: &str, mailboxes: &[(String, Option<u32>)]) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            Self::set_memberships_tx(&tx, message_id, mailboxes)?;
            tx.execute("UPDATE messages SET version=version+1, mirrored=0 WHERE id=?", [message_id])?;
            tx.commit()?;
            Ok(())
        })?;
        if let Some(t) = self.thread_of(message_id)? {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    pub fn memberships(&self, message_id: &str) -> Result<Vec<(String, Option<u32>)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT mailbox_id, uid FROM message_mailboxes WHERE message_id=?")?;
            let rows = s.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn set_flags(&self, message_id: &str, flags: &Flags) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE messages SET seen=?, flagged=?, answered=?, draft=?, version=version+1, mirrored=0 WHERE id=?",
                params![flags.seen, flags.flagged, flags.answered, flags.draft, message_id],
            )?;
            Ok(())
        })?;
        if let Some(t) = self.thread_of(message_id)? {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    pub fn set_blob(&self, message_id: &str, hash: &str, snippet: &str, attachments: &[Attachment]) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE messages SET blob_hash=?, snippet=CASE WHEN ?='' THEN snippet ELSE ? END, attachments_json=?, has_attachments=?, indexed=0, mirrored=0 WHERE id=?",
                params![hash, snippet, snippet, to_json(&attachments), attachments.iter().any(|a| !a.inline), message_id],
            )?;
            Ok(())
        })?;
        if let Some(t) = self.thread_of(message_id)? {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    pub fn delete_message(&self, message_id: &str) -> Result<()> {
        let thread = self.thread_of(message_id)?;
        self.w(|c| {
            c.execute("DELETE FROM messages WHERE id=?", [message_id])?;
            Ok(())
        })?;
        if let Some(t) = thread {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    /// Remove a message from one mailbox; deletes it when no memberships remain.
    pub fn remove_from_mailbox(&self, message_id: &str, mailbox_id: &str) -> Result<()> {
        let thread = self.thread_of(message_id)?;
        self.w(|c| {
            c.execute("DELETE FROM message_mailboxes WHERE message_id=? AND mailbox_id=?", [message_id, mailbox_id])?;
            c.execute(
                "DELETE FROM messages WHERE id=? AND NOT EXISTS (SELECT 1 FROM message_mailboxes WHERE message_id=?)",
                [message_id, message_id],
            )?;
            Ok(())
        })?;
        if let Some(t) = thread {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    /// (uid, message id, flags) for every message in a mailbox.
    pub fn mailbox_uids(&self, mailbox_id: &str) -> Result<Vec<(u32, String, Flags)>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT mm.uid, m.id, m.seen, m.flagged, m.answered, m.draft FROM message_mailboxes mm JOIN messages m ON m.id=mm.message_id WHERE mm.mailbox_id=? AND mm.uid IS NOT NULL",
            )?;
            let rows = s
                .query_map([mailbox_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, Flags { seen: r.get(2)?, flagged: r.get(3)?, answered: r.get(4)?, draft: r.get(5)? }))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Messages placed in a mailbox optimistically (no server UID yet).
    pub fn mailbox_unconfirmed(&self, mailbox_id: &str) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT message_id FROM message_mailboxes WHERE mailbox_id=? AND uid IS NULL")?;
            let rows = s.query_map([mailbox_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    /// Remote ids of all messages in an account (for full-resync diffs).
    pub fn remote_ids(&self, account_id: &str) -> Result<HashMap<String, String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT remote_id, id FROM messages WHERE account_id=?")?;
            let rows = s.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
            Ok(rows)
        })
    }

    pub fn thread_of(&self, message_id: &str) -> Result<Option<String>> {
        self.r(|c| Ok(c.query_row("SELECT thread_id FROM messages WHERE id=?", [message_id], |r| r.get(0)).optional()?))
    }

    pub fn message_ref(&self, message_id: &str) -> Result<Option<MessageRef>> {
        let base = self.r(|c| {
            Ok(c.query_row(
                "SELECT id, account_id, remote_id, remote_thread_id, message_id_hdr, seen, flagged, answered, draft, blob_hash, version FROM messages WHERE id=?",
                [message_id],
                |r| {
                    Ok(MessageRef {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        remote_id: r.get(2)?,
                        remote_thread_id: r.get(3)?,
                        message_id_hdr: r.get(4)?,
                        mailboxes: vec![],
                        flags: Flags { seen: r.get(5)?, flagged: r.get(6)?, answered: r.get(7)?, draft: r.get(8)? },
                        blob_hash: r.get(9)?,
                        version: r.get(10)?,
                    })
                },
            )
            .optional()?)
        })?;
        let Some(mut m) = base else { return Ok(None) };
        m.mailboxes = self.r(|c| {
            let mut s = c.prepare(
                "SELECT mm.mailbox_id, mb.remote_id, mm.uid FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=?",
            )?;
            let rows = s.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        Ok(Some(m))
    }

    fn summary_row(r: &Row) -> rusqlite::Result<MessageSummary> {
        let from_email: Option<String> = r.get("from_email")?;
        Ok(MessageSummary {
            id: r.get("id")?,
            account_id: r.get("account_id")?,
            thread_id: r.get("thread_id")?,
            subject: r.get("subject")?,
            from: from_email.map(|e| Address { name: r.get("from_name").ok().flatten(), email: e }),
            to: from_json(&r.get::<_, String>("to_json")?),
            date: ms_to_dt(r.get("date")?),
            snippet: r.get("snippet")?,
            flags: Flags { seen: r.get("seen")?, flagged: r.get("flagged")?, answered: r.get("answered")?, draft: r.get("draft")? },
            mailbox_ids: vec![],
            has_attachments: r.get("has_attachments")?,
        })
    }

    fn fill_mailbox_ids(c: &Connection, msgs: &mut [MessageSummary]) -> Result<()> {
        let mut s = c.prepare_cached("SELECT mailbox_id FROM message_mailboxes WHERE message_id=?")?;
        for m in msgs.iter_mut() {
            m.mailbox_ids = s.query_map([&m.id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        }
        Ok(())
    }

    /// Everything stored about a message except bodies.
    pub fn message_row(&self, id: &str) -> Result<Option<StoredMessage>> {
        self.r(|c| {
            let row = c
                .query_row("SELECT * FROM messages WHERE id=?", [id], |r| {
                    Ok(StoredMessage {
                        summary: Self::summary_row(r)?,
                        cc: from_json(&r.get::<_, String>("cc_json")?),
                        bcc: from_json(&r.get::<_, String>("bcc_json")?),
                        reply_to: from_json(&r.get::<_, String>("reply_to_json")?),
                        references: from_json(&r.get::<_, String>("refs_json")?),
                        message_id_hdr: r.get("message_id_hdr")?,
                        in_reply_to: r.get("in_reply_to")?,
                        attachments: from_json(&r.get::<_, String>("attachments_json")?),
                        blob_hash: r.get("blob_hash")?,
                        remote_id: r.get("remote_id")?,
                        version: r.get("version")?,
                    })
                })
                .optional()?;
            let Some(mut row) = row else { return Ok(None) };
            Self::fill_mailbox_ids(c, std::slice::from_mut(&mut row.summary))?;
            Ok(Some(row))
        })
    }

    pub fn thread_message_ids(&self, thread_id: &str) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT id FROM messages WHERE thread_id=? ORDER BY date, id")?;
            let rows = s.query_map([thread_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn message_version(&self, id: &str) -> Result<Option<i64>> {
        self.r(|c| Ok(c.query_row("SELECT version FROM messages WHERE id=?", [id], |r| r.get(0)).optional()?))
    }

    /// Messages whose body still needs fetching, newest first.
    pub fn messages_without_body(&self, account_id: &str, since: DateTime<Utc>, limit: u32) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT id FROM messages WHERE account_id=? AND blob_hash IS NULL AND date>=? ORDER BY date DESC LIMIT ?",
            )?;
            let rows = s
                .query_map(params![account_id, since.timestamp_millis(), limit], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn unindexed(&self, limit: u32) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT id FROM messages WHERE indexed=0 AND blob_hash IS NOT NULL LIMIT ?")?;
            let rows = s.query_map([limit], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn mark_indexed(&self, ids: &[String]) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            for id in ids {
                tx.execute("UPDATE messages SET indexed=1 WHERE id=?", [id])?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn unmirrored_threads(&self, limit: u32) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT DISTINCT thread_id FROM messages WHERE mirrored=0 LIMIT ?")?;
            let rows = s.query_map([limit], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn mark_thread_mirrored(&self, thread_id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE messages SET mirrored=1 WHERE thread_id=?", [thread_id])?;
            Ok(())
        })
    }

    pub fn reset_mirrored(&self) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE messages SET mirrored=0", [])?;
            Ok(())
        })
    }

    pub fn reset_indexed(&self) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE messages SET indexed=0", [])?;
            Ok(())
        })
    }

    pub fn known_senders(&self, prefix: &str, limit: u32) -> Result<Vec<Address>> {
        self.r(|c| {
            let like = format!("{}%", prefix.to_lowercase());
            let mut s = c.prepare(
                "SELECT from_name, from_email, COUNT(*) n FROM messages WHERE from_email IS NOT NULL AND (lower(from_email) LIKE ?1 OR lower(from_name) LIKE ?1)
                 GROUP BY lower(from_email) ORDER BY n DESC LIMIT ?2",
            )?;
            let rows = s
                .query_map(params![like, limit], |r| Ok(Address { name: r.get(0)?, email: r.get(1)? }))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    // -----------------------------------------------------------------------
    // Threads
    // -----------------------------------------------------------------------

    pub fn recompute_thread(&self, thread_id: &str) -> Result<()> {
        self.w(|c| {
            let mut s = c.prepare(
                "SELECT id, account_id, subject, date, seen, flagged, has_attachments, snippet, from_name, from_email, to_json, message_id_hdr, draft
                 FROM messages WHERE thread_id=? ORDER BY date",
            )?;
            struct M {
                id: String,
                account: String,
                subject: String,
                date: i64,
                seen: bool,
                flagged: bool,
                att: bool,
                snippet: String,
                from: Option<Address>,
                to: Vec<Address>,
                mid: Option<String>,
            }
            let rows: Vec<M> = s
                .query_map([thread_id], |r| {
                    let fe: Option<String> = r.get(9)?;
                    Ok(M {
                        id: r.get(0)?,
                        account: r.get(1)?,
                        subject: r.get(2)?,
                        date: r.get(3)?,
                        seen: r.get(4)?,
                        flagged: r.get(5)?,
                        att: r.get(6)?,
                        snippet: r.get(7)?,
                        from: fe.map(|e| Address { name: r.get(8).ok().flatten(), email: e }),
                        to: from_json(&r.get::<_, String>(10)?),
                        mid: r.get(11)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(s);
            if rows.is_empty() {
                c.execute("DELETE FROM threads WHERE id=?", [thread_id])?;
                return Ok(());
            }
            // Count duplicate copies (same Message-ID in several folders) once.
            let mut seen_mids = HashSet::new();
            let unique: Vec<&M> = rows
                .iter()
                .filter(|m| m.mid.as_ref().map(|x| seen_mids.insert(x.clone())).unwrap_or(true))
                .collect();
            let last = rows.last().unwrap();
            let first = &rows[0];
            let mut participants: Vec<Address> = vec![];
            for m in &unique {
                if let Some(f) = &m.from
                    && !participants.iter().any(|p| p.email.eq_ignore_ascii_case(&f.email)) {
                        participants.push(f.clone());
                    }
            }
            if participants.is_empty() {
                participants = first.to.clone();
            }
            let subject = if first.subject.is_empty() { last.subject.clone() } else { first.subject.clone() };
            c.execute(
                "INSERT INTO threads (id, account_id, subject, last_date, message_count, unread_count, flagged, has_attachments, snippet, latest_message_id, participants_json)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET subject=excluded.subject, last_date=excluded.last_date, message_count=excluded.message_count,
                   unread_count=excluded.unread_count, flagged=excluded.flagged, has_attachments=excluded.has_attachments,
                   snippet=excluded.snippet, latest_message_id=excluded.latest_message_id, participants_json=excluded.participants_json",
                params![
                    thread_id,
                    last.account,
                    subject,
                    last.date,
                    unique.len() as i64,
                    unique.iter().filter(|m| !m.seen).count() as i64,
                    rows.iter().any(|m| m.flagged),
                    rows.iter().any(|m| m.att),
                    last.snippet,
                    last.id,
                    to_json(&participants)
                ],
            )?;
            Ok(())
        })
    }

    fn thread_row(r: &Row) -> rusqlite::Result<ThreadSummary> {
        Ok(ThreadSummary {
            thread_id: r.get("id")?,
            account_id: r.get("account_id")?,
            subject: r.get("subject")?,
            participants: from_json(&r.get::<_, String>("participants_json")?),
            last_date: ms_to_dt(r.get("last_date")?),
            message_count: r.get("message_count")?,
            unread_count: r.get("unread_count")?,
            flagged: r.get("flagged")?,
            has_attachments: r.get("has_attachments")?,
            snippet: r.get("snippet")?,
            mailbox_ids: vec![],
            latest_message_id: r.get("latest_message_id")?,
        })
    }

    /// List threads with structured filters. `ids` restricts to a candidate
    /// set of message ids (from full-text search).
    pub fn list_threads(&self, f: &ThreadFilter) -> Result<Vec<ThreadSummary>> {
        self.r(|c| {
            let mut where_: Vec<String> = vec![];
            let mut p: Vec<rusqlite::types::Value> = vec![];
            use rusqlite::types::Value as V;
            let mut msg_conds: Vec<String> = vec![];
            // Small mailboxes: drive from the membership index (avoids scanning
            // every thread). Large ones: walk threads newest-first and stop early.
            let total_threads: i64 = c.query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0))?;
            let size: i64 = match (&f.mailbox_id, f.role) {
                (Some(mb), _) => c.query_row("SELECT total FROM mailboxes WHERE id=?", [mb], |r| r.get(0)).unwrap_or(0),
                (None, Some(role)) => c.query_row("SELECT COALESCE(SUM(total),0) FROM mailboxes WHERE role=?", [role_str(role)], |r| r.get(0)).unwrap_or(0),
                _ => 0,
            };
            let sparse = size * 10 < total_threads;
            // Clauses (and their params) that precede the message-level EXISTS.
            let mut pre_where: Vec<String> = vec![];
            let mut pre_p: Vec<rusqlite::types::Value> = vec![];
            if let Some(mb) = &f.mailbox_id {
                if sparse {
                    pre_where.push("t.id IN (SELECT m2.thread_id FROM message_mailboxes mm2 JOIN messages m2 ON m2.id=mm2.message_id WHERE mm2.mailbox_id=?)".into());
                    pre_p.push(V::Text(mb.clone()));
                } else {
                    msg_conds.push("EXISTS (SELECT 1 FROM message_mailboxes mm WHERE mm.message_id=m.id AND mm.mailbox_id=?)".into());
                    p.push(V::Text(mb.clone()));
                }
            }
            if let Some(role) = f.role {
                if sparse {
                    pre_where.push("t.id IN (SELECT m2.thread_id FROM message_mailboxes mm2 JOIN messages m2 ON m2.id=mm2.message_id WHERE mm2.mailbox_id IN (SELECT id FROM mailboxes WHERE role=?))".into());
                    pre_p.push(V::Text(role_str(role).into()));
                } else {
                    msg_conds.push("EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=m.id AND mb.role=?)".into());
                    p.push(V::Text(role_str(role).into()));
                }
            }
            if let Some(names) = &f.in_mailbox_names {
                let ph = vec!["?"; names.len()].join(",");
                msg_conds.push(format!(
                    "EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=m.id AND (lower(mb.name) IN ({ph}) OR mb.role IN ({ph})))"
                ));
                for n in names {
                    p.push(V::Text(n.to_lowercase()));
                }
                for n in names {
                    p.push(V::Text(n.to_lowercase()));
                }
            }
            if f.exclude_trash_junk {
                msg_conds.push("NOT EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=m.id AND mb.role IN ('trash','junk'))".into());
            }
            if let Some(a) = &f.account_ids {
                let ph = vec!["?"; a.len()].join(",");
                msg_conds.push(format!("m.account_id IN ({ph})"));
                for x in a {
                    p.push(V::Text(x.clone()));
                }
            }
            if let Some(from) = &f.from {
                msg_conds.push("(lower(m.from_email) LIKE ? OR lower(m.from_name) LIKE ?)".into());
                let like = format!("%{}%", from.to_lowercase());
                p.push(V::Text(like.clone()));
                p.push(V::Text(like));
            }
            if let Some(to) = &f.to {
                msg_conds.push("(lower(m.to_json) LIKE ? OR lower(m.cc_json) LIKE ?)".into());
                let like = format!("%{}%", to.to_lowercase());
                p.push(V::Text(like.clone()));
                p.push(V::Text(like));
            }
            if let Some(s) = &f.subject {
                msg_conds.push("lower(m.subject) LIKE ?".into());
                p.push(V::Text(format!("%{}%", s.to_lowercase())));
            }
            if f.has_attachment {
                msg_conds.push("m.has_attachments=1".into());
            }
            if f.unread {
                msg_conds.push("m.seen=0".into());
            }
            if f.flagged {
                msg_conds.push("m.flagged=1".into());
            }
            if let Some(d) = f.after {
                msg_conds.push("m.date>=?".into());
                p.push(V::Integer(d.timestamp_millis()));
            }
            if let Some(d) = f.msg_before {
                msg_conds.push("m.date<?".into());
                p.push(V::Integer(d.timestamp_millis()));
            }
            if let Some(ids) = &f.message_ids {
                if ids.is_empty() {
                    return Ok(vec![]);
                }
                let ph = vec!["?"; ids.len()].join(",");
                msg_conds.push(format!("m.id IN ({ph})"));
                for x in ids {
                    p.push(V::Text(x.clone()));
                }
            }
            if !msg_conds.is_empty() {
                where_.push(format!("EXISTS (SELECT 1 FROM messages m WHERE m.thread_id=t.id AND {})", msg_conds.join(" AND ")));
            }
            if let Some(b) = f.before {
                where_.push("t.last_date<?".into());
                p.push(V::Integer(b.timestamp_millis()));
            }
            pre_where.append(&mut where_);
            let where_ = pre_where;
            pre_p.append(&mut p);
            let mut p = pre_p;
            let sql = format!(
                "SELECT t.* FROM threads t {} ORDER BY t.last_date DESC LIMIT ?",
                if where_.is_empty() { String::new() } else { format!("WHERE {}", where_.join(" AND ")) }
            );
            p.push(V::Integer(f.limit as i64));
            let mut s = c.prepare(&sql)?;
            let mut rows = s.query_map(params_from_iter(p.iter()), Self::thread_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut ms = c.prepare_cached(
                "SELECT DISTINCT mm.mailbox_id FROM messages m JOIN message_mailboxes mm ON mm.message_id=m.id WHERE m.thread_id=?",
            )?;
            for t in rows.iter_mut() {
                t.mailbox_ids = ms.query_map([&t.thread_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            }
            Ok(rows)
        })
    }

    pub fn get_thread_summary(&self, id: &str) -> Result<Option<ThreadSummary>> {
        self.r(|c| {
            let Some(mut t) = c.query_row("SELECT * FROM threads WHERE id=?", [id], Self::thread_row).optional()? else { return Ok(None) };
            let mut s = c.prepare_cached(
                "SELECT DISTINCT mm.mailbox_id FROM messages m JOIN message_mailboxes mm ON mm.message_id=m.id WHERE m.thread_id=?",
            )?;
            t.mailbox_ids = s.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(Some(t))
        })
    }

    pub fn mirror_path(&self, thread_id: &str) -> Result<Option<String>> {
        self.r(|c| Ok(c.query_row("SELECT path FROM mirror_paths WHERE thread_id=?", [thread_id], |r| r.get(0)).optional()?))
    }

    pub fn set_mirror_path(&self, thread_id: &str, path: Option<&str>) -> Result<()> {
        self.w(|c| {
            match path {
                Some(p) => c.execute(
                    "INSERT INTO mirror_paths (thread_id, path) VALUES (?,?) ON CONFLICT(thread_id) DO UPDATE SET path=excluded.path",
                    params![thread_id, p],
                )?,
                None => c.execute("DELETE FROM mirror_paths WHERE thread_id=?", [thread_id])?,
            };
            Ok(())
        })
    }

    /// Mirror entries whose thread no longer exists.
    pub fn orphan_mirror_paths(&self) -> Result<Vec<(String, String)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT thread_id, path FROM mirror_paths WHERE thread_id NOT IN (SELECT id FROM threads)")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    // -----------------------------------------------------------------------
    // Drafts
    // -----------------------------------------------------------------------

    pub fn save_draft(&self, d: &Draft) -> Result<String> {
        let id = d.id.clone().unwrap_or_else(new_id);
        let mut d = d.clone();
        d.id = Some(id.clone());
        self.w(|c| {
            c.execute(
                "INSERT INTO drafts (id, account_id, draft_json, updated_at) VALUES (?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id, draft_json=excluded.draft_json, updated_at=excluded.updated_at",
                params![id, d.account_id, to_json(&d), now_ms()],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    pub fn get_draft(&self, id: &str) -> Result<Option<(Draft, Option<String>)>> {
        self.r(|c| {
            Ok(c.query_row("SELECT draft_json, server_message_id FROM drafts WHERE id=?", [id], |r| {
                let j: String = r.get(0)?;
                Ok((serde_json::from_str(&j).unwrap_or_default(), r.get(1)?))
            })
            .optional()?)
        })
    }

    pub fn set_draft_server_message(&self, id: &str, message_id: Option<&str>) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE drafts SET server_message_id=? WHERE id=?", params![message_id, id])?;
            Ok(())
        })
    }

    pub fn delete_draft(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM drafts WHERE id=?", [id])?;
            Ok(())
        })
    }

    pub fn list_drafts(&self) -> Result<Vec<Draft>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT draft_json FROM drafts ORDER BY updated_at DESC")?;
            let rows = s
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|j| j.ok().and_then(|j| serde_json::from_str(&j).ok()))
                .collect();
            Ok(rows)
        })
    }

    // -----------------------------------------------------------------------
    // Ops
    // -----------------------------------------------------------------------

    pub fn enqueue_op(&self, account_id: &str, kind: &str, payload: &serde_json::Value, actor: &str) -> Result<String> {
        let id = new_id();
        self.w(|c| {
            c.execute(
                "INSERT INTO ops (id, account_id, kind, payload_json, status, actor, created_at) VALUES (?,?,?,?, 'pending', ?, ?)",
                params![id, account_id, kind, payload.to_string(), actor, now_ms()],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    pub fn due_ops(&self, account_id: &str) -> Result<Vec<OpRow>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT id, account_id, kind, payload_json, attempts, actor FROM ops WHERE account_id=? AND status='pending' AND next_attempt<=? ORDER BY created_at",
            )?;
            let rows = s
                .query_map(params![account_id, now_ms()], |r| {
                    Ok(OpRow {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        kind: r.get(2)?,
                        payload: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                        attempts: r.get(4)?,
                        actor: r.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn has_pending_ops(&self, account_id: &str) -> Result<bool> {
        self.r(|c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM ops WHERE account_id=? AND status='pending'", [account_id], |r| r.get(0))?;
            Ok(n > 0)
        })
    }

    pub fn op_done(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE ops SET status='done' WHERE id=?", [id])?;
            // Keep a short history only.
            c.execute("DELETE FROM ops WHERE status='done' AND created_at<?", [now_ms() - 7 * 86_400_000])?;
            Ok(())
        })
    }

    pub fn op_retry(&self, id: &str, error: &str, attempts: u32) -> Result<()> {
        let backoff = (2i64.pow(attempts.min(10)) * 1000).min(15 * 60 * 1000);
        self.w(|c| {
            c.execute(
                "UPDATE ops SET attempts=?, last_error=?, next_attempt=? WHERE id=?",
                params![attempts, error, now_ms() + backoff, id],
            )?;
            Ok(())
        })
    }

    pub fn op_failed(&self, id: &str, error: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE ops SET status='failed', last_error=? WHERE id=?", params![error, id])?;
            Ok(())
        })
    }

    pub fn op_reset(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE ops SET status='pending', attempts=0, next_attempt=0 WHERE id=?", [id])?;
            Ok(())
        })
    }

    pub fn list_outbox(&self) -> Result<Vec<OutboxItem>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT id, account_id, kind, status, attempts, last_error, created_at FROM ops WHERE status<>'done' ORDER BY created_at DESC LIMIT 200",
            )?;
            let rows = s
                .query_map([], |r| {
                    Ok(OutboxItem {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        kind: r.get(2)?,
                        status: r.get(3)?,
                        attempts: r.get(4)?,
                        last_error: r.get(5)?,
                        created_at: ms_to_dt(r.get(6)?),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    // -----------------------------------------------------------------------
    // Calendars & events
    // -----------------------------------------------------------------------

    pub fn upsert_calendar(&self, account_id: &str, remote_id: &str, name: &str, color: Option<&str>, read_only: bool) -> Result<String> {
        self.w(|c| {
            let existing: Option<String> = c
                .query_row("SELECT id FROM calendars WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?;
            if let Some(id) = existing {
                c.execute(
                    "UPDATE calendars SET name=?, read_only=?, color=COALESCE(?, color) WHERE id=?",
                    params![name, read_only, color, id],
                )?;
                return Ok(id);
            }
            let n: i64 = c.query_row("SELECT COUNT(*) FROM calendars", [], |r| r.get(0))?;
            let id = new_id();
            let col = color.map(|s| s.to_string()).unwrap_or_else(|| PALETTE[(n as usize + 2) % PALETTE.len()].to_string());
            c.execute(
                "INSERT INTO calendars (id, account_id, remote_id, name, color, read_only) VALUES (?,?,?,?,?,?)",
                params![id, account_id, remote_id, name, col, read_only],
            )?;
            Ok(id)
        })
    }

    fn calendar_row(r: &Row) -> rusqlite::Result<StoredCalendar> {
        Ok(StoredCalendar {
            cal: Calendar {
                id: r.get("id")?,
                account_id: r.get("account_id")?,
                name: r.get("name")?,
                color: r.get("color")?,
                read_only: r.get("read_only")?,
                visible: r.get("visible")?,
            },
            remote_id: r.get("remote_id")?,
            sync_token: r.get("sync_token")?,
        })
    }

    pub fn list_calendars(&self, account_id: Option<&str>) -> Result<Vec<StoredCalendar>> {
        self.r(|c| {
            let rows = match account_id {
                Some(a) => {
                    let mut s = c.prepare("SELECT * FROM calendars WHERE account_id=? ORDER BY name")?;
                    s.query_map([a], Self::calendar_row)?.collect::<rusqlite::Result<Vec<_>>>()?
                }
                None => {
                    let mut s = c.prepare("SELECT * FROM calendars ORDER BY account_id, name")?;
                    s.query_map([], Self::calendar_row)?.collect::<rusqlite::Result<Vec<_>>>()?
                }
            };
            Ok(rows)
        })
    }

    pub fn get_calendar(&self, id: &str) -> Result<Option<StoredCalendar>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM calendars WHERE id=?", [id], Self::calendar_row).optional()?))
    }

    pub fn set_calendar_visible(&self, id: &str, visible: bool) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE calendars SET visible=? WHERE id=?", params![visible, id])?;
            Ok(())
        })
    }

    pub fn set_calendar_token(&self, id: &str, token: Option<&str>) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE calendars SET sync_token=? WHERE id=?", params![token, id])?;
            Ok(())
        })
    }

    pub fn prune_calendars(&self, account_id: &str, keep_remote: &[String]) -> Result<()> {
        let all = self.list_calendars(Some(account_id))?;
        self.w(|c| {
            for cal in all {
                if !keep_remote.contains(&cal.remote_id) {
                    c.execute("DELETE FROM calendars WHERE id=?", [&cal.cal.id])?;
                }
            }
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_event(
        &self,
        calendar_id: &str,
        account_id: &str,
        remote_id: &str,
        etag: Option<&str>,
        uid: &str,
        ics: &str,
        title: &str,
        range: (DateTime<Utc>, DateTime<Utc>),
        recurring: bool,
        pending: bool,
    ) -> Result<String> {
        self.w(|c| {
            let existing: Option<String> = c
                .query_row("SELECT id FROM events WHERE calendar_id=? AND remote_id=?", [calendar_id, remote_id], |r| r.get(0))
                .optional()?;
            if let Some(id) = existing {
                c.execute(
                    "UPDATE events SET etag=?, uid=?, ics=?, title=?, range_start=?, range_end=?, is_recurring=?, pending=?, version=version+1 WHERE id=?",
                    params![etag, uid, ics, title, range.0.timestamp_millis(), range.1.timestamp_millis(), recurring, pending, id],
                )?;
                return Ok(id);
            }
            let id = new_id();
            c.execute(
                "INSERT INTO events (id, calendar_id, account_id, remote_id, etag, uid, ics, title, range_start, range_end, is_recurring, pending)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                params![id, calendar_id, account_id, remote_id, etag, uid, ics, title, range.0.timestamp_millis(), range.1.timestamp_millis(), recurring, pending],
            )?;
            Ok(id)
        })
    }

    fn event_row(r: &Row) -> rusqlite::Result<StoredEvent> {
        Ok(StoredEvent {
            id: r.get("id")?,
            calendar_id: r.get("calendar_id")?,
            account_id: r.get("account_id")?,
            remote_id: r.get("remote_id")?,
            etag: r.get("etag")?,
            uid: r.get("uid")?,
            ics: r.get("ics")?,
            version: r.get("version")?,
            pending: r.get("pending")?,
        })
    }

    pub fn get_event(&self, id: &str) -> Result<Option<StoredEvent>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM events WHERE id=?", [id], Self::event_row).optional()?))
    }

    pub fn event_by_remote(&self, calendar_id: &str, remote_id: &str) -> Result<Option<StoredEvent>> {
        self.r(|c| {
            Ok(c.query_row("SELECT * FROM events WHERE calendar_id=? AND remote_id=?", [calendar_id, remote_id], Self::event_row)
                .optional()?)
        })
    }

    pub fn events_by_uid(&self, account_id: &str, uid: &str) -> Result<Vec<StoredEvent>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM events WHERE account_id=? AND uid=?")?;
            let rows = s.query_map([account_id, uid], Self::event_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn events_in_range(&self, from: DateTime<Utc>, to: DateTime<Utc>, calendar_ids: Option<&[String]>) -> Result<Vec<StoredEvent>> {
        self.r(|c| {
            let mut sql = String::from("SELECT * FROM events WHERE range_start<? AND range_end>?");
            let mut p: Vec<rusqlite::types::Value> =
                vec![to.timestamp_millis().into(), from.timestamp_millis().into()];
            if let Some(ids) = calendar_ids {
                if ids.is_empty() {
                    return Ok(vec![]);
                }
                sql.push_str(&format!(" AND calendar_id IN ({})", vec!["?"; ids.len()].join(",")));
                for i in ids {
                    p.push(i.clone().into());
                }
            }
            let mut s = c.prepare(&sql)?;
            let rows = s.query_map(params_from_iter(p.iter()), Self::event_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn event_remote_ids(&self, calendar_id: &str) -> Result<HashMap<String, (String, Option<String>)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT remote_id, id, etag FROM events WHERE calendar_id=? AND pending=0")?;
            let rows = s
                .query_map([calendar_id], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
                .collect::<rusqlite::Result<HashMap<_, _>>>()?;
            Ok(rows)
        })
    }

    pub fn set_event_remote(&self, id: &str, remote_id: &str, etag: Option<&str>) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE events SET remote_id=?, etag=?, pending=0 WHERE id=?",
                params![remote_id, etag, id],
            )?;
            Ok(())
        })
    }

    pub fn update_event_ics(&self, id: &str, ics: &str, title: &str, range: (DateTime<Utc>, DateTime<Utc>), recurring: bool) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE events SET ics=?, title=?, range_start=?, range_end=?, is_recurring=?, pending=1, version=version+1 WHERE id=?",
                params![ics, title, range.0.timestamp_millis(), range.1.timestamp_millis(), recurring, id],
            )?;
            Ok(())
        })
    }

    pub fn delete_event(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM events WHERE id=?", [id])?;
            Ok(())
        })
    }

    pub fn delete_event_by_remote(&self, calendar_id: &str, remote_id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM events WHERE calendar_id=? AND remote_id=?", [calendar_id, remote_id])?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Agents
    // -----------------------------------------------------------------------

    pub fn insert_agent(&self, name: &str, token_hash: &str, policy: &AgentPolicy) -> Result<Agent> {
        let id = new_id();
        self.w(|c| {
            c.execute(
                "INSERT INTO agents (id, name, token_hash, created_at, policy_json) VALUES (?,?,?,?,?)",
                params![id, name, token_hash, now_ms(), to_json(policy)],
            )?;
            Ok(())
        })?;
        self.get_agent(&id)?.ok_or_else(|| anyhow!("agent vanished"))
    }

    fn agent_row(r: &Row) -> rusqlite::Result<Agent> {
        Ok(Agent {
            id: r.get("id")?,
            name: r.get("name")?,
            created_at: ms_to_dt(r.get("created_at")?),
            last_used: r.get::<_, Option<i64>>("last_used")?.map(ms_to_dt),
            policy: serde_json::from_str(&r.get::<_, String>("policy_json")?).unwrap_or_default(),
            revoked: r.get("revoked")?,
        })
    }

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM agents ORDER BY created_at")?;
            let rows = s.query_map([], Self::agent_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM agents WHERE id=?", [id], Self::agent_row).optional()?))
    }

    pub fn agent_by_token_hash(&self, hash: &str) -> Result<Option<Agent>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM agents WHERE token_hash=? AND revoked=0", [hash], Self::agent_row).optional()?))
    }

    pub fn update_agent(&self, id: &str, name: Option<&str>, policy: Option<&AgentPolicy>) -> Result<()> {
        self.w(|c| {
            if let Some(n) = name {
                c.execute("UPDATE agents SET name=? WHERE id=?", params![n, id])?;
            }
            if let Some(p) = policy {
                c.execute("UPDATE agents SET policy_json=? WHERE id=?", params![to_json(p), id])?;
            }
            Ok(())
        })
    }

    pub fn touch_agent(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE agents SET last_used=? WHERE id=?", params![now_ms(), id])?;
            Ok(())
        })
    }

    pub fn revoke_agent(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE agents SET revoked=1 WHERE id=?", [id])?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Proposals
    // -----------------------------------------------------------------------

    pub fn insert_proposal(&self, p: &Proposal, versions: &HashMap<String, i64>) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO proposals (id, agent_id, created_at, status, action_json, rationale, source_threads_json, warnings_json, summary, versions_json)
                 VALUES (?,?,?,?,?,?,?,?,?,?)",
                params![
                    p.id, p.agent_id, p.created_at.timestamp_millis(), proposal_status_str(p.status), to_json(&p.action), p.rationale,
                    to_json(&p.source_thread_ids), to_json(&p.warnings), p.summary, to_json(versions)
                ],
            )?;
            Ok(())
        })
    }

    fn proposal_row(r: &Row) -> rusqlite::Result<(Proposal, HashMap<String, i64>)> {
        let action: ProposalAction = serde_json::from_str(&r.get::<_, String>("action_json")?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok((
            Proposal {
                id: r.get("id")?,
                agent_id: r.get("agent_id")?,
                agent_name: r.get::<_, Option<String>>("agent_name").ok().flatten().unwrap_or_default(),
                created_at: ms_to_dt(r.get("created_at")?),
                status: parse_proposal_status(&r.get::<_, String>("status")?),
                action,
                rationale: r.get("rationale")?,
                source_thread_ids: from_json(&r.get::<_, String>("source_threads_json")?),
                warnings: from_json(&r.get::<_, String>("warnings_json")?),
                review_note: r.get("review_note")?,
                decided_at: r.get::<_, Option<i64>>("decided_at")?.map(ms_to_dt),
                summary: r.get("summary")?,
            },
            from_json(&r.get::<_, String>("versions_json")?),
        ))
    }

    pub fn list_proposals(&self, status: Option<ProposalStatus>, agent_id: Option<&str>) -> Result<Vec<Proposal>> {
        self.r(|c| {
            let mut sql = String::from(
                "SELECT p.*, a.name AS agent_name FROM proposals p LEFT JOIN agents a ON a.id=p.agent_id WHERE 1=1",
            );
            let mut p: Vec<String> = vec![];
            if let Some(s) = status {
                sql.push_str(" AND p.status=?");
                p.push(proposal_status_str(s).into());
            }
            if let Some(a) = agent_id {
                sql.push_str(" AND p.agent_id=?");
                p.push(a.into());
            }
            sql.push_str(" ORDER BY p.created_at DESC LIMIT 500");
            let mut s = c.prepare(&sql)?;
            let rows = s
                .query_map(params_from_iter(p.iter()), Self::proposal_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().map(|x| x.0).collect())
        })
    }

    pub fn get_proposal(&self, id: &str) -> Result<Option<(Proposal, HashMap<String, i64>)>> {
        self.r(|c| {
            Ok(c.query_row(
                "SELECT p.*, a.name AS agent_name FROM proposals p LEFT JOIN agents a ON a.id=p.agent_id WHERE p.id=?",
                [id],
                Self::proposal_row,
            )
            .optional()?)
        })
    }

    pub fn set_proposal_status(&self, id: &str, status: ProposalStatus, note: Option<&str>, action: Option<&ProposalAction>) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE proposals SET status=?, review_note=COALESCE(?, review_note), decided_at=?, action_json=COALESCE(?, action_json) WHERE id=?",
                params![proposal_status_str(status), note, now_ms(), action.map(to_json), id],
            )?;
            Ok(())
        })
    }

    pub fn proposals_since(&self, agent_id: &str, since: DateTime<Utc>) -> Result<u32> {
        self.r(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM proposals WHERE agent_id=? AND created_at>=?",
                params![agent_id, since.timestamp_millis()],
                |r| r.get(0),
            )?)
        })
    }

    // -----------------------------------------------------------------------
    // Audit & settings
    // -----------------------------------------------------------------------

    pub fn audit(&self, actor: &str, action: &str, detail: &serde_json::Value) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO audit (at, actor, action, detail_json) VALUES (?,?,?,?)",
                params![now_ms(), actor, action, detail.to_string()],
            )?;
            Ok(())
        })
    }

    pub fn list_audit(&self, limit: u32) -> Result<Vec<AuditEntry>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT id, at, actor, action, detail_json FROM audit ORDER BY id DESC LIMIT ?")?;
            let rows = s
                .query_map([limit], |r| {
                    Ok(AuditEntry {
                        id: r.get(0)?,
                        at: ms_to_dt(r.get(1)?),
                        actor: r.get(2)?,
                        action: r.get(3)?,
                        detail: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.r(|c| {
            let v: Option<String> = c.query_row("SELECT value FROM settings WHERE key=?", [key], |r| r.get(0)).optional()?;
            Ok(v.and_then(|v| serde_json::from_str(&v).ok()))
        })
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, to_json(value)],
            )?;
            Ok(())
        })
    }
}

#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub summary: MessageSummary,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub references: Vec<String>,
    pub message_id_hdr: Option<String>,
    pub in_reply_to: Option<String>,
    pub attachments: Vec<Attachment>,
    pub blob_hash: Option<String>,
    pub remote_id: String,
    pub version: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ThreadFilter {
    pub mailbox_id: Option<String>,
    pub role: Option<MailboxRole>,
    pub account_ids: Option<Vec<String>>,
    pub in_mailbox_names: Option<Vec<String>>,
    pub exclude_trash_junk: bool,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub has_attachment: bool,
    pub unread: bool,
    pub flagged: bool,
    pub after: Option<DateTime<Utc>>,
    pub msg_before: Option<DateTime<Utc>>,
    pub message_ids: Option<Vec<String>>,
    pub before: Option<DateTime<Utc>>,
    pub limit: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(remote: &str, mid: &str, irt: Option<&str>, subject: &str, mins: i64) -> MessageRecord {
        MessageRecord {
            remote_id: remote.into(),
            message_id_hdr: Some(mid.into()),
            in_reply_to: irt.map(|s| s.into()),
            references: irt.map(|s| vec![s.to_string()]).unwrap_or_default(),
            subject: subject.into(),
            from: Some(Address::new("bob@x.test")),
            date: Utc::now() + chrono::Duration::minutes(mins),
            snippet: subject.into(),
            ..Default::default()
        }
    }

    #[test]
    fn threading_out_of_order() {
        let s = Store::open_in_memory().unwrap();
        let acc = s
            .insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() })
            .unwrap();
        let mb = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
        // Child arrives first, then an unrelated message, then the parent.
        let (c, _) = s.upsert_message(&acc.id, &rec("2", "m2", Some("m1"), "Re: hi", 1), &[(mb.clone(), Some(2))]).unwrap();
        let (u, _) = s.upsert_message(&acc.id, &rec("3", "m3", None, "other", 2), &[(mb.clone(), Some(3))]).unwrap();
        let (p, _) = s.upsert_message(&acc.id, &rec("1", "m1", None, "hi", 0), &[(mb.clone(), Some(1))]).unwrap();
        let tc = s.thread_of(&c).unwrap().unwrap();
        assert_eq!(s.thread_of(&p).unwrap().unwrap(), tc);
        assert_ne!(s.thread_of(&u).unwrap().unwrap(), tc);
        let t = s.get_thread_summary(&tc).unwrap().unwrap();
        assert_eq!(t.message_count, 2);
        assert_eq!(t.unread_count, 2);
        assert_eq!(t.subject, "hi");

        let list = s.list_threads(&ThreadFilter { mailbox_id: Some(mb.clone()), limit: 10, ..Default::default() }).unwrap();
        assert_eq!(list.len(), 2);
        // Newest first: the unrelated message is the most recent.
        assert_eq!(list[0].thread_id, s.thread_of(&u).unwrap().unwrap());
        assert_eq!(list[1].thread_id, tc);
    }
}
