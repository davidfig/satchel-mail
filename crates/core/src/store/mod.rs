//! SQLite-backed store. All SQL lives in this module (and `schema.sql`) so a
//! different backend (Postgres for the hosted version) can be swapped in
//! behind the same API. One file per table group, each adding to `impl Store`.

mod accounts;
mod agents;
mod audit;
mod bodies;
mod calendars;
mod contacts;
mod drafts;
mod mailboxes;
mod message_reads;
mod messages;
mod ops;
mod people;
mod sender_rules;
mod smart_folders;
#[cfg(test)]
mod tests;
mod threads;
mod types;

pub use bodies::StoredBlob;
pub use types::*;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use satchel_api::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Rows (id, date, e) for each message you sent, once per lowercase recipient
/// address (To, Cc or Bcc). Drafts are left out.
const SENT_TO: &str = "SELECT DISTINCT m.id, m.date, lower(json_extract(j.value,'$.email')) e
     FROM messages m JOIN accounts a ON a.id=m.account_id AND m.from_email=a.email COLLATE NOCASE,
          json_each(json_array(json(m.to_json), json(m.cc_json), json(m.bcc_json))) l, json_each(l.value) j
     WHERE m.draft=0";

/// Remote id prefix of a folder created locally and not yet on the server.
pub const PENDING_MAILBOX: &str = "pending:";

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

fn verdict_str(v: SenderVerdict) -> &'static str {
    match v {
        SenderVerdict::Spam => "spam",
        SenderVerdict::NotSpam => "not_spam",
    }
}

fn parse_verdict(s: &str) -> SenderVerdict {
    if s == "spam" { SenderVerdict::Spam } else { SenderVerdict::NotSpam }
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

fn origin_str(o: ContactOrigin) -> &'static str {
    match o {
        ContactOrigin::Auto => "auto",
        ContactOrigin::Local => "local",
        ContactOrigin::Remote => "remote",
    }
}

fn parse_origin(s: &str) -> ContactOrigin {
    match s {
        "auto" => ContactOrigin::Auto,
        "remote" => ContactOrigin::Remote,
        _ => ContactOrigin::Local,
    }
}

const PALETTE: &[&str] = &["#4f7cff", "#e5484d", "#30a46c", "#f76b15", "#8e4ec6", "#12a594", "#d6409f", "#978365"];

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

/// The schema version `schema.sql` creates. To change the schema, edit
/// `schema.sql` to the new shape, bump this, and append to [`MIGRATIONS`] the
/// SQL that takes a database from the previous version to this one.
const SCHEMA_VERSION: i64 = 3;

/// `MIGRATIONS[i]` upgrades a version `i + 1` database to version `i + 2`.
const MIGRATIONS: &[&str] = &[
    // 2: attachment-free copies and per-account storage limits (0036).
    "ALTER TABLE messages ADD COLUMN partial INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE messages ADD COLUMN blob_size INTEGER NOT NULL DEFAULT 0;",
    // 3: smart folders (0039).
    "CREATE TABLE IF NOT EXISTS smart_folders (
        id          TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        query       TEXT NOT NULL,
        badge       TEXT NOT NULL DEFAULT 'unread',
        position    INTEGER NOT NULL,
        created_at  INTEGER NOT NULL
    );",
];

/// Create or upgrade the schema, tracked in `PRAGMA user_version`.
fn migrate(c: &mut Connection) -> Result<()> {
    const _: () = assert!(MIGRATIONS.len() as i64 == SCHEMA_VERSION - 1);
    let mut version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 0 {
        // A new database gets the current schema. One from before versioning
        // has tables already: it is version 1 (the schema only grew new
        // tables until then, which `schema.sql` adds).
        let has_tables: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='accounts')", [], |r| r.get(0))?;
        let tx = c.transaction()?;
        tx.execute_batch(include_str!("schema.sql"))?;
        version = if has_tables { 1 } else { SCHEMA_VERSION };
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    if version > SCHEMA_VERSION {
        return Err(anyhow!("this database was created by a newer version of Satchel (schema {version}, this version reads up to {SCHEMA_VERSION})"));
    }
    while version < SCHEMA_VERSION {
        let tx = c.transaction()?;
        tx.execute_batch(MIGRATIONS[(version - 1) as usize]).with_context(|| format!("migrating the database to schema {}", version + 1))?;
        version += 1;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut w = open_conn(path)?;
        migrate(&mut w)?;
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
        let mut w = w;
        w.execute_batch("PRAGMA foreign_keys=ON;")?;
        migrate(&mut w)?;
        let r = Connection::open_with_flags(
            &name,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        // Shared-cache connections take table locks and fail with "table is
        // locked" instead of waiting; letting the reader skip them avoids that.
        r.execute_batch("PRAGMA read_uncommitted=1;")?;
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
}
