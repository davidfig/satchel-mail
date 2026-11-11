//! Proposals between computers (0080, phase 2).
//!
//! A proposal points at messages, mailboxes, calendars and events by local
//! id, which differ on each computer. In the sync folder each id is written
//! as the account's email and the provider's own id (plus the Message-ID
//! header for messages), and read back as this computer's id. A proposal
//! whose targets haven't synced here yet waits and is tried again.
//!
//! Only the computer where it's approved carries it out; the others take the
//! outcome. Contact proposals stay on the computer that made them: contacts
//! made from mail have no id another computer shares.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Duration, Utc};
use satchel_api::*;
use serde_json::{Value, json};

use crate::Engine;
use crate::store::ms_to_dt;

/// Proposals sync for this long after they're made (and are then forgotten
/// by the sync, not deleted).
pub const KEEP_DAYS: i64 = 60;
/// A draft carrying more than this in attachments stays on its computer.
const MAX_ATTACHMENTS: usize = 256 * 1024;
const FOLDER_AGENT: &str = "builtin:ai-folder";

/// What applying another computer's proposal did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Its targets aren't here yet: try again next pass.
    Later,
    /// This computer's version is the one to keep (a decision here beats an
    /// older "pending" from elsewhere): send it out again.
    Mine,
}

#[derive(Clone, Copy)]
enum Kind {
    Account,
    Message,
    Mailbox,
    Calendar,
    Event,
}

pub fn cutoff() -> chrono::DateTime<Utc> {
    Utc::now() - Duration::days(KEEP_DAYS)
}

fn decided(s: ProposalStatus) -> bool {
    matches!(s, ProposalStatus::Approved | ProposalStatus::Applied | ProposalStatus::Rejected | ProposalStatus::Failed)
}

/// This computer's recent proposals as sync items.
pub fn local(engine: &Engine, out: &mut std::collections::BTreeMap<String, Value>) -> Result<()> {
    let store = engine.store();
    let emails = account_emails(engine)?;
    let builtin: std::collections::HashSet<String> = store.list_agents()?.into_iter().filter(|a| a.builtin).map(|a| a.id).collect();
    for p in store.proposals_created_since(cutoff())? {
        let Some(action) = map_action(&p.action, &mut |k, id| to_ref(engine, &emails, k, id))? else { continue };
        let sources: Vec<String> = p
            .source_thread_ids
            .iter()
            .filter_map(|t| store.thread_latest_message(t).ok().flatten())
            .filter_map(|m| to_ref(engine, &emails, Kind::Message, &m).ok().flatten())
            .collect();
        out.insert(
            format!("proposal:{}", p.id),
            json!({
                "agent": if builtin.contains(&p.agent_id) { FOLDER_AGENT.to_string() } else { p.agent_id.clone() },
                "created_at": p.created_at.timestamp_millis(),
                "status": p.status,
                "action": action,
                "rationale": p.rationale,
                "sources": sources,
                "warnings": p.warnings,
                "review_note": p.review_note,
                "decided_at": p.decided_at.map(|d| d.timestamp_millis()),
                "summary": p.summary,
            }),
        );
    }
    Ok(())
}

/// Puts another computer's proposal into effect here: adds it, or takes its
/// outcome. Never carries out the action.
pub fn apply(engine: &Engine, id: &str, v: &Value) -> Result<Outcome> {
    // Proposals aren't removed; a tombstone is one the sync stopped carrying.
    if v.is_null() {
        return Ok(Outcome::Done);
    }
    let store = engine.store();
    let ids = account_ids(engine)?;
    let portable: ProposalAction = serde_json::from_value(v["action"].clone())?;
    let Some(action) = map_action(&portable, &mut |k, r| from_ref(engine, &ids, k, r))? else { return Ok(Outcome::Later) };
    let status: ProposalStatus = serde_json::from_value(v["status"].clone())?;
    let ms = |k: &str| v[k].as_i64().map(ms_to_dt);

    let Some((mine, _)) = store.get_proposal(id)? else {
        let agent_id = match v["agent"].as_str().unwrap_or_default() {
            FOLDER_AGENT => store.builtin_agent("ai-folder", "AI folder", &AgentPolicy::default())?.id,
            a => a.to_string(),
        };
        let sources = v["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r.as_str())
            .filter_map(|r| from_ref(engine, &ids, Kind::Message, r).ok().flatten())
            .filter_map(|m| store.message_thread(&m).ok().flatten())
            .collect();
        let p = Proposal {
            id: id.into(),
            agent_id,
            agent_name: String::new(),
            created_at: ms("created_at").unwrap_or_else(Utc::now),
            status,
            action,
            rationale: v["rationale"].as_str().unwrap_or_default().into(),
            source_thread_ids: sources,
            warnings: serde_json::from_value(v["warnings"].clone()).unwrap_or_default(),
            review_note: v["review_note"].as_str().map(String::from),
            decided_at: ms("decided_at"),
            summary: v["summary"].as_str().unwrap_or_default().into(),
        };
        // Staleness here is measured from when it arrived here.
        store.insert_synced_proposal(&p, &engine.target_versions(&p.action)?)?;
        engine.emit(CoreEvent::ProposalsChanged);
        return Ok(Outcome::Done);
    };

    if decided(mine.status) && !decided(status) {
        return Ok(Outcome::Mine);
    }
    let theirs = Proposal { status, action, review_note: v["review_note"].as_str().map(String::from), decided_at: ms("decided_at"), ..mine.clone() };
    if serde_json::to_value(&theirs)? != serde_json::to_value(&mine)? {
        let edited = serde_json::to_value(&theirs.action)? != serde_json::to_value(&mine.action)?;
        store.set_synced_proposal_outcome(&theirs)?;
        engine.proposal_decided(id, edited);
        engine.emit(CoreEvent::ProposalsChanged);
    }
    Ok(Outcome::Done)
}

fn account_emails(engine: &Engine) -> Result<HashMap<String, String>> {
    Ok(engine.store().list_accounts()?.into_iter().map(|a| (a.id, a.email.to_lowercase())).collect())
}

fn account_ids(engine: &Engine) -> Result<HashMap<String, String>> {
    Ok(engine.store().list_accounts()?.into_iter().map(|a| (a.email.to_lowercase(), a.id)).collect())
}

/// A local id as the computers share it, e.g. `["m", email, remote id, Message-ID]`.
fn to_ref(engine: &Engine, emails: &HashMap<String, String>, k: Kind, id: &str) -> Result<Option<String>> {
    let s = engine.store();
    let email = |acct: &str| emails.get(acct).cloned();
    let r = match k {
        Kind::Account => email(id).map(|e| json!(["a", e])),
        Kind::Message => s.message_sync_ref(id)?.and_then(|(a, rid, hdr)| Some(json!(["m", email(&a)?, rid, hdr]))),
        Kind::Mailbox => s.mailbox_ref(id)?.and_then(|(a, rid)| Some(json!(["b", email(&a)?, rid]))),
        Kind::Calendar => s.calendar_ref(id)?.and_then(|(a, rid)| Some(json!(["c", email(&a)?, rid]))),
        Kind::Event => match s.event_ref(id)? {
            Some((cal, rid)) => s.calendar_ref(&cal)?.and_then(|(a, crid)| Some(json!(["e", email(&a)?, crid, rid]))),
            None => None,
        },
    };
    Ok(r.map(|v| v.to_string()))
}

/// This computer's id for a shared one; None if it isn't here (yet).
fn from_ref(engine: &Engine, ids: &HashMap<String, String>, k: Kind, r: &str) -> Result<Option<String>> {
    let s = engine.store();
    let v: Vec<Value> = serde_json::from_str(r)?;
    let at = |i: usize| v.get(i).and_then(|x| x.as_str()).unwrap_or_default();
    let Some(acct) = ids.get(&at(1).to_lowercase()) else { return Ok(None) };
    let expect = match k {
        Kind::Account => "a",
        Kind::Message => "m",
        Kind::Mailbox => "b",
        Kind::Calendar => "c",
        Kind::Event => "e",
    };
    if at(0) != expect {
        anyhow::bail!("a {expect} reference was expected");
    }
    Ok(match k {
        Kind::Account => Some(acct.clone()),
        Kind::Message => s.message_by_ref(acct, at(2), v.get(3).and_then(|x| x.as_str()))?,
        Kind::Mailbox => s.mailbox_id_by_remote(acct, at(2))?,
        Kind::Calendar => s.calendar_id_by_remote(acct, at(2))?,
        Kind::Event => match s.calendar_id_by_remote(acct, at(2))? {
            Some(cal) => s.event_id_by_remote(&cal, at(3))?,
            None => None,
        },
    })
}

/// The action with every id passed through `f`. None if `f` has no answer
/// for one, or the action doesn't travel (contacts, large attachments).
fn map_action(a: &ProposalAction, f: &mut dyn FnMut(Kind, &str) -> Result<Option<String>>) -> Result<Option<ProposalAction>> {
    let mut a = a.clone();
    let mut missing = false;
    let mut one = |k: Kind, s: &mut String| -> Result<()> {
        if !missing {
            match f(k, s)? {
                Some(n) => *s = n,
                None => missing = true,
            }
        }
        Ok(())
    };
    match &mut a {
        ProposalAction::Draft { draft } => {
            if draft.attachments.iter().map(|x| x.data_base64.len()).sum::<usize>() > MAX_ATTACHMENTS {
                return Ok(None);
            }
            draft.id = None;
            one(Kind::Account, &mut draft.account_id)?;
            for m in draft.in_reply_to.iter_mut().chain(draft.forward_of.iter_mut()) {
                one(Kind::Message, m)?;
            }
        }
        ProposalAction::Labels { message_ids, add, remove } => {
            for m in message_ids {
                one(Kind::Message, m)?;
            }
            for b in add.iter_mut().chain(remove.iter_mut()) {
                one(Kind::Mailbox, b)?;
            }
        }
        ProposalAction::Archive { message_ids }
        | ProposalAction::MarkRead { message_ids, .. }
        | ProposalAction::Flag { message_ids, .. } => {
            for m in message_ids {
                one(Kind::Message, m)?;
            }
        }
        ProposalAction::Move { message_ids, mailbox_id } => {
            for m in message_ids {
                one(Kind::Message, m)?;
            }
            one(Kind::Mailbox, mailbox_id)?;
        }
        ProposalAction::CreateEvent { event } => {
            event.id = None;
            one(Kind::Calendar, &mut event.calendar_id)?;
        }
        ProposalAction::UpdateEvent { event } => {
            one(Kind::Calendar, &mut event.calendar_id)?;
            if let Some(id) = &mut event.id {
                one(Kind::Event, id)?;
            }
        }
        ProposalAction::CancelEvent { event_id } | ProposalAction::RespondToInvite { event_id, .. } => one(Kind::Event, event_id)?,
        ProposalAction::SaveContact { .. } | ProposalAction::MergeContacts { .. } => return Ok(None),
    }
    Ok((!missing).then_some(a))
}
