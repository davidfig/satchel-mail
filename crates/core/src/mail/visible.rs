//! What a caller with a date limit may see of a thread: its messages inside
//! the limit, and a summary built from only those, so no listing counts,
//! names or previews an older message. Used for agents' thread lists and for
//! the AI folder's indexes.

use std::collections::HashSet;

use anyhow::Result;
use chrono::{DateTime, Utc};
use satchel_api::*;

use crate::store::{Store, StoredMessage};

/// The thread's messages dated `cutoff` or later (all of them without one),
/// oldest first.
pub(crate) fn visible_messages(store: &Store, thread_id: &str, cutoff: Option<DateTime<Utc>>) -> Result<Vec<StoredMessage>> {
    let mut msgs = vec![];
    for id in store.thread_message_ids(thread_id)? {
        if let Some(m) = store.message_row(&id)?.filter(|m| cutoff.is_none_or(|c| m.summary.date >= c)) {
            msgs.push(m);
        }
    }
    Ok(msgs)
}

/// `t` as a caller with this date limit may see it: unchanged without one,
/// else rebuilt from its messages inside the limit, or None if there are none.
pub(crate) fn visible_summary(store: &Store, t: ThreadSummary, cutoff: Option<DateTime<Utc>>) -> Result<Option<ThreadSummary>> {
    if cutoff.is_none() {
        return Ok(Some(t));
    }
    let msgs = visible_messages(store, &t.thread_id, cutoff)?;
    Ok(summarize(t, &msgs))
}

/// `t` rebuilt from `msgs` (oldest first) the way the store builds it from
/// all of them: copies of one message count once, the subject is the first
/// message's, the preview the last one's. None if `msgs` is empty.
pub(crate) fn summarize(t: ThreadSummary, msgs: &[StoredMessage]) -> Option<ThreadSummary> {
    let (first, last) = (msgs.first()?, msgs.last()?);
    let mut seen = HashSet::new();
    let unique: Vec<&StoredMessage> = msgs.iter().filter(|m| m.message_id_hdr.as_ref().is_none_or(|h| seen.insert(h.clone()))).collect();
    let mut participants: Vec<Address> = vec![];
    for a in unique.iter().filter_map(|m| m.summary.from.as_ref()) {
        if !participants.iter().any(|p| p.email.eq_ignore_ascii_case(&a.email)) {
            participants.push(a.clone());
        }
    }
    if participants.is_empty() {
        participants = first.summary.to.clone();
    }
    let mut mailbox_ids: Vec<Id> = vec![];
    for id in msgs.iter().flat_map(|m| m.summary.mailbox_ids.iter()) {
        if !mailbox_ids.contains(id) {
            mailbox_ids.push(id.clone());
        }
    }
    Some(ThreadSummary {
        subject: if first.summary.subject.is_empty() { last.summary.subject.clone() } else { first.summary.subject.clone() },
        participants,
        last_date: last.summary.date,
        message_count: unique.len() as u32,
        unread_count: unique.iter().filter(|m| !m.summary.flags.seen).count() as u32,
        flagged: msgs.iter().any(|m| m.summary.flags.flagged),
        has_attachments: msgs.iter().any(|m| m.summary.has_attachments),
        snippet: last.summary.snippet.clone(),
        mailbox_ids,
        latest_message_id: last.summary.id.clone(),
        ..t
    })
}
