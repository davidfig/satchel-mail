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
mod favorites;
mod mailboxes;
mod message_reads;
mod messages;
mod ops;
mod people;
mod sender_rules;
#[cfg(test)]
mod tests;
mod threads;
mod types;

pub use bodies::StoredBlob;
pub use types::*;

use std::cell::Cell;
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
/// address (To, Cc or Bcc). Drafts are left out, and so is mail outside a
/// Sent folder: anyone can put your address in From.
const SENT_TO: &str = "SELECT DISTINCT m.id, m.date, lower(json_extract(j.value,'$.email')) e
     FROM messages m JOIN accounts a ON a.id=m.account_id AND m.from_email=a.email COLLATE NOCASE,
          json_each(json_array(json(m.to_json), json(m.cc_json), json(m.bcc_json))) l, json_each(l.value) j
     WHERE m.draft=0 AND EXISTS (SELECT 1 FROM message_mailboxes sm JOIN mailboxes sb ON sb.id=sm.mailbox_id
                                 WHERE sm.message_id=m.id AND sb.role='sent')";

/// Whether a failed write was down to the data being written (too big, a
/// constraint, a value SQLite won't take) rather than the database itself
/// (busy, locked, I/O, disk full, corrupt). Sync skips a message that fails
/// for the first kind of reason instead of stopping on it.
pub fn is_data_error(e: &anyhow::Error) -> bool {
    use rusqlite::ErrorCode as C;
    let by_code = |f: &rusqlite::ffi::Error, msg: Option<&str>| match f.code {
        C::TooBig | C::ConstraintViolation | C::TypeMismatch | C::ParameterOutOfRange => true,
        // Plain SQLITE_ERROR: only the limits a message can hit.
        C::Unknown => msg.is_some_and(|m| m.contains("too many") || m.contains("too big") || m.contains("too long")),
        _ => false,
    };
    e.chain().filter_map(|c| c.downcast_ref::<rusqlite::Error>()).any(|e| match e {
        rusqlite::Error::SqliteFailure(f, msg) => by_code(f, msg.as_deref()),
        rusqlite::Error::SqlInputError { error, msg, .. } => by_code(error, Some(msg)),
        rusqlite::Error::ToSqlConversionFailure(_) | rusqlite::Error::FromSqlConversionFailure(..) | rusqlite::Error::IntegralValueOutOfRange(..) => true,
        _ => false,
    })
}

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

/// Reads go through one of three connections, each behind its own lock, so a
/// slow background read never makes the user wait (0038).
pub struct Store {
    write: Mutex<Connection>,
    /// The user's requests (UI, CLI, MCP).
    read: Mutex<Connection>,
    /// Background workers, inside [`background`].
    bg_read: Mutex<Connection>,
    /// Only `PRAGMA data_version`, polled by an async task that mustn't block.
    watch: Mutex<Connection>,
    /// Blob hashes of deleted messages, for the engine to delete once unused.
    freed_blobs: Mutex<Vec<String>>,
}

thread_local! {
    static BACKGROUND: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` with this thread's reads on the background connection. For
/// blocking work (`spawn_blocking`) that stays on one thread.
pub fn background<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            BACKGROUND.with(|b| b.set(self.0));
        }
    }
    let _restore = Restore(BACKGROUND.with(|b| b.replace(true)));
    f()
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
const SCHEMA_VERSION: i64 = 6;

/// `MIGRATIONS[i]` upgrades a version `i + 1` database to version `i + 2`.
const MIGRATIONS: &[&str] = &[
    // 2: attachment-free copies and per-account storage limits (0036).
    "ALTER TABLE messages ADD COLUMN partial INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE messages ADD COLUMN blob_size INTEGER NOT NULL DEFAULT 0;",
    // 3: the processed file each folder proposal's status goes to (0040).
    "ALTER TABLE proposals ADD COLUMN processed_file TEXT;",
    // 4: each message's authserv-id, read lazily, for trusting Authentication-Results.
    "ALTER TABLE messages ADD COLUMN auth_serv TEXT;",
    // 5: each event's location, for suggesting places (0050); filled in Rust below.
    "ALTER TABLE events ADD COLUMN location TEXT NOT NULL DEFAULT '';",
    // 6: favorite senders (0053).
    "CREATE TABLE IF NOT EXISTS favorite_senders (email TEXT PRIMARY KEY, created_at INTEGER NOT NULL);",
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
        // SQL can't read iCalendar, so the new column is filled here.
        if version == 5 {
            calendars::fill_event_locations(&tx)?;
        }
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
        let r = || open_conn(path).map(Mutex::new);
        Ok(Self { write: Mutex::new(w), read: r()?, bg_read: r()?, watch: r()?, freed_blobs: Mutex::default() })
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
        let r = || -> Result<Mutex<Connection>> {
            let r = Connection::open_with_flags(
                &name,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            )?;
            // Shared-cache connections take table locks and fail with "table is
            // locked" instead of waiting; letting readers skip them avoids that.
            r.execute_batch("PRAGMA read_uncommitted=1;")?;
            Ok(Mutex::new(r))
        };
        Ok(Self { write: Mutex::new(w), read: r()?, bg_read: r()?, watch: r()?, freed_blobs: Mutex::default() })
    }

    fn w<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut c = self.write.lock().map_err(|_| anyhow!("store poisoned"))?;
        f(&mut c)
    }

    fn r<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = if BACKGROUND.with(Cell::get) { &self.bg_read } else { &self.read };
        let c = conn.lock().map_err(|_| anyhow!("store poisoned"))?;
        f(&c)
    }

    /// Changes whenever another connection (or process) commits.
    pub fn data_version(&self) -> Result<i64> {
        let c = self.watch.lock().map_err(|_| anyhow!("store poisoned"))?;
        Ok(c.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }
}
