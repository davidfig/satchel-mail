-- Satchel Mail local store, schema version 2.
-- Every account-scoped row carries account_id; accounts carry tenant_id so the
-- same schema can be multi-tenant in the hosted version.

CREATE TABLE IF NOT EXISTS accounts (
    id            TEXT PRIMARY KEY,
    tenant_id     TEXT NOT NULL DEFAULT 'local',
    kind          TEXT NOT NULL,
    email         TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    config_json   TEXT NOT NULL,
    color         TEXT NOT NULL,
    sync_state    TEXT NOT NULL DEFAULT 'idle',
    last_success  INTEGER,
    error         TEXT,
    detail        TEXT,
    created_at    INTEGER NOT NULL
);

-- Folders and labels.
CREATE TABLE IF NOT EXISTS mailboxes (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id   TEXT NOT NULL,
    name        TEXT NOT NULL,
    path        TEXT NOT NULL,
    role        TEXT NOT NULL,
    is_label    INTEGER NOT NULL DEFAULT 0,
    unread      INTEGER NOT NULL DEFAULT 0,
    total       INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_id, remote_id)
);

CREATE TABLE IF NOT EXISTS messages (
    id               TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id        TEXT NOT NULL,
    remote_thread_id TEXT,
    thread_id        TEXT NOT NULL,
    message_id_hdr   TEXT,
    in_reply_to      TEXT,
    subject          TEXT NOT NULL DEFAULT '',
    from_name        TEXT,
    from_email       TEXT,
    to_json          TEXT NOT NULL DEFAULT '[]',
    cc_json          TEXT NOT NULL DEFAULT '[]',
    bcc_json         TEXT NOT NULL DEFAULT '[]',
    reply_to_json    TEXT NOT NULL DEFAULT '[]',
    refs_json        TEXT NOT NULL DEFAULT '[]',
    date             INTEGER NOT NULL,
    snippet          TEXT NOT NULL DEFAULT '',
    seen             INTEGER NOT NULL DEFAULT 0,
    flagged          INTEGER NOT NULL DEFAULT 0,
    answered         INTEGER NOT NULL DEFAULT 0,
    draft            INTEGER NOT NULL DEFAULT 0,
    has_attachments  INTEGER NOT NULL DEFAULT 0,
    attachments_json TEXT NOT NULL DEFAULT '[]',
    blob_hash        TEXT,
    size             INTEGER NOT NULL DEFAULT 0,
    -- The stored copy leaves out attachment contents (fetched when asked for).
    partial          INTEGER NOT NULL DEFAULT 0,
    -- Bytes the stored copy takes on disk (0: unknown, use size).
    blob_size        INTEGER NOT NULL DEFAULT 0,
    -- Local version, bumped on every change (used for proposal staleness).
    version          INTEGER NOT NULL DEFAULT 1,
    indexed          INTEGER NOT NULL DEFAULT 0,
    mirrored         INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_id, remote_id)
);
CREATE INDEX IF NOT EXISTS messages_thread ON messages(thread_id, date);
CREATE INDEX IF NOT EXISTS messages_msgid ON messages(account_id, message_id_hdr);
CREATE INDEX IF NOT EXISTS messages_date ON messages(account_id, date);
CREATE INDEX IF NOT EXISTS messages_unindexed ON messages(indexed) WHERE indexed = 0;
CREATE INDEX IF NOT EXISTS messages_unmirrored ON messages(mirrored) WHERE mirrored = 0;
CREATE INDEX IF NOT EXISTS messages_from ON messages(account_id, from_email COLLATE NOCASE);

-- Mailbox membership; uid is the IMAP UID (or NULL for label-based providers).
CREATE TABLE IF NOT EXISTS message_mailboxes (
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    mailbox_id  TEXT NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
    uid         INTEGER,
    PRIMARY KEY (message_id, mailbox_id)
);
CREATE INDEX IF NOT EXISTS mm_mailbox ON message_mailboxes(mailbox_id, uid);

-- References/In-Reply-To ids, for threading lookups in both directions.
CREATE TABLE IF NOT EXISTS message_refs (
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    account_id  TEXT NOT NULL,
    ref         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS refs_ref ON message_refs(account_id, ref);
CREATE INDEX IF NOT EXISTS refs_msg ON message_refs(message_id);

-- Denormalized thread summaries, recomputed when member messages change.
CREATE TABLE IF NOT EXISTS threads (
    id                 TEXT PRIMARY KEY,
    account_id         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject            TEXT NOT NULL,
    last_date          INTEGER NOT NULL,
    message_count      INTEGER NOT NULL,
    unread_count       INTEGER NOT NULL,
    flagged            INTEGER NOT NULL,
    has_attachments    INTEGER NOT NULL,
    snippet            TEXT NOT NULL,
    latest_message_id  TEXT NOT NULL,
    participants_json  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS threads_date ON threads(last_date);
-- Keyset paging: (last_date, id) gives a total order.
CREATE INDEX IF NOT EXISTS threads_date_id ON threads(last_date, id);
CREATE INDEX IF NOT EXISTS threads_account_date ON threads(account_id, last_date);

-- Per-account sync cursors (historyId, deltaLink, JMAP state, modseq...).
CREATE TABLE IF NOT EXISTS sync_cursors (
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    PRIMARY KEY (account_id, key)
);

CREATE TABLE IF NOT EXISTS drafts (
    id                  TEXT PRIMARY KEY,
    account_id          TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    draft_json          TEXT NOT NULL,
    -- Local message id of the server-side copy, if synced to the Drafts folder.
    server_message_id   TEXT,
    updated_at          INTEGER NOT NULL
);

-- Durable queue of mutations to push to providers.
CREATE TABLE IF NOT EXISTS ops (
    id           TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status       TEXT NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    actor        TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    next_attempt INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ops_pending ON ops(status, next_attempt);

CREATE TABLE IF NOT EXISTS calendars (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id   TEXT NOT NULL,
    name        TEXT NOT NULL,
    color       TEXT NOT NULL,
    read_only   INTEGER NOT NULL DEFAULT 0,
    visible     INTEGER NOT NULL DEFAULT 1,
    sync_token  TEXT,
    UNIQUE (account_id, remote_id)
);

CREATE TABLE IF NOT EXISTS events (
    id           TEXT PRIMARY KEY,
    calendar_id  TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    account_id   TEXT NOT NULL,
    remote_id    TEXT NOT NULL,
    etag         TEXT,
    uid          TEXT NOT NULL,
    ics          TEXT NOT NULL,
    title        TEXT NOT NULL,
    -- Bounds of the whole series, for range queries.
    range_start  INTEGER NOT NULL,
    range_end    INTEGER NOT NULL,
    is_recurring INTEGER NOT NULL DEFAULT 0,
    version      INTEGER NOT NULL DEFAULT 1,
    -- Created locally and not yet confirmed by the server.
    pending      INTEGER NOT NULL DEFAULT 0,
    UNIQUE (calendar_id, remote_id)
);
CREATE INDEX IF NOT EXISTS events_range ON events(range_start, range_end);
CREATE INDEX IF NOT EXISTS events_uid ON events(account_id, uid);

-- People. `origin` is auto (generated from mail and untouched by the user),
-- local (created or edited here) or remote (from a server address book).
-- Deleted contacts stay as tombstones, with their addresses, so automatic
-- generation never brings them back.
CREATE TABLE IF NOT EXISTS contacts (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL DEFAULT '',
    fields_json TEXT NOT NULL,
    origin      TEXT NOT NULL,
    deleted     INTEGER NOT NULL DEFAULT 0,
    version     INTEGER NOT NULL DEFAULT 1,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Every address of every contact (lowercase), for lookups by address.
CREATE TABLE IF NOT EXISTS contact_emails (
    contact_id  TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    email       TEXT NOT NULL,
    PRIMARY KEY (contact_id, email)
);
CREATE INDEX IF NOT EXISTS contact_emails_email ON contact_emails(email);

CREATE TABLE IF NOT EXISTS address_books (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id   TEXT NOT NULL,
    name        TEXT NOT NULL,
    read_only   INTEGER NOT NULL DEFAULT 0,
    sync_token  TEXT,
    UNIQUE (account_id, remote_id)
);

-- A contact's copy in a server address book. `vcard` is the last version the
-- server confirmed: the base for merging server changes with local edits.
-- remote_id starts with "local:" until the card is first pushed.
CREATE TABLE IF NOT EXISTS contact_cards (
    id               TEXT PRIMARY KEY,
    contact_id       TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    address_book_id  TEXT NOT NULL REFERENCES address_books(id) ON DELETE CASCADE,
    account_id       TEXT NOT NULL,
    remote_id        TEXT NOT NULL,
    etag             TEXT,
    uid              TEXT NOT NULL,
    vcard            TEXT NOT NULL,
    UNIQUE (address_book_id, remote_id)
);
CREATE INDEX IF NOT EXISTS contact_cards_contact ON contact_cards(contact_id);

CREATE TABLE IF NOT EXISTS agents (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    token_hash  TEXT NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL,
    last_used   INTEGER,
    policy_json TEXT NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS proposals (
    id                   TEXT PRIMARY KEY,
    agent_id             TEXT NOT NULL,
    created_at           INTEGER NOT NULL,
    status               TEXT NOT NULL,
    action_json          TEXT NOT NULL,
    rationale            TEXT NOT NULL,
    source_threads_json  TEXT NOT NULL,
    warnings_json        TEXT NOT NULL,
    review_note          TEXT,
    decided_at           INTEGER,
    summary              TEXT NOT NULL,
    -- {target id: version} at proposal time, for staleness checks.
    versions_json        TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS proposals_status ON proposals(status, created_at);

CREATE TABLE IF NOT EXISTS audit (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at          INTEGER NOT NULL,
    actor       TEXT NOT NULL,
    action      TEXT NOT NULL,
    detail_json TEXT NOT NULL
);

-- Standing spam decisions by sender address (stored lowercase). Enforced
-- after every sync, so server filters can't put the sender back.
CREATE TABLE IF NOT EXISTS sender_rules (
    email       TEXT PRIMARY KEY,
    verdict     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

-- Saved searches shown under Mail in the sidebar (0039). Local only.
CREATE TABLE IF NOT EXISTS smart_folders (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    query       TEXT NOT NULL,
    badge       TEXT NOT NULL DEFAULT 'unread',
    position    INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

-- Where each thread was written in the AI mirror, for cleanup after merges/deletes.
CREATE TABLE IF NOT EXISTS mirror_paths (
    thread_id  TEXT PRIMARY KEY,
    path       TEXT NOT NULL
);

-- Full-text search over messages (contentless: text lives in blobs/messages).
-- Keyed by messages.rowid; never VACUUM without rebuilding this index.
CREATE VIRTUAL TABLE IF NOT EXISTS message_fts USING fts5(
    subject, participants, body, attachments,
    content='', contentless_delete=1,
    tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS messages_fts_delete AFTER DELETE ON messages BEGIN
    DELETE FROM message_fts WHERE rowid = old.rowid;
END;
