//! Markdown proposal files → proposals.

use super::*;

pub(crate) fn from_markdown(store: &Store, text: &str) -> Result<DroppedProposal> {
    let (fm, body) = parse(text)?;
    let body = body.trim().to_string();
    let action_name = fm.str("action").ok_or_else(|| anyhow!("missing `action:`"))?.to_lowercase().replace('-', "_");
    let threads: Vec<String> = fm.list("thread").into_iter().chain(fm.list("threads")).collect();
    let messages = || -> Result<Vec<String>> {
        let mut ids: Vec<String> = fm.list("message").into_iter().chain(fm.list("messages")).collect();
        for t in &threads {
            let found = store.thread_message_ids(t)?;
            if found.is_empty() {
                bail!("unknown thread {t}");
            }
            ids.extend(found);
        }
        if ids.is_empty() {
            bail!("give `threads:` or `messages:`");
        }
        ids.dedup();
        Ok(ids)
    };
    let action = match action_name.as_str() {
        "reply" => ProposalAction::Draft { draft: reply(store, &fm, &threads, body)? },
        "draft" | "new" | "compose" => {
            let account_id = account(store, fm.str("account"))?;
            ProposalAction::Draft {
                draft: Draft {
                    account_id,
                    to: addresses(&fm, "to")?,
                    cc: addresses(&fm, "cc")?,
                    bcc: addresses(&fm, "bcc")?,
                    subject: fm.str("subject").unwrap_or_default().to_string(),
                    body_text: body,
                    ..Default::default()
                },
            }
        }
        "archive" => ProposalAction::Archive { message_ids: messages()? },
        "move" => {
            let ids = messages()?;
            let to = fm.str("mailbox").or(fm.str("to")).ok_or_else(|| anyhow!("missing `mailbox:`"))?;
            ProposalAction::Move { mailbox_id: mailbox(store, &ids[0], to)?, message_ids: ids }
        }
        "labels" | "label" => {
            let ids = messages()?;
            let resolve = |key: &str| fm.list(key).iter().map(|l| mailbox(store, &ids[0], l)).collect::<Result<Vec<_>>>();
            ProposalAction::Labels { add: resolve("add")?, remove: resolve("remove")?, message_ids: ids }
        }
        "mark_read" => ProposalAction::MarkRead { message_ids: messages()?, seen: true },
        "mark_unread" => ProposalAction::MarkRead { message_ids: messages()?, seen: false },
        "flag" => ProposalAction::Flag { message_ids: messages()?, flagged: true },
        "unflag" => ProposalAction::Flag { message_ids: messages()?, flagged: false },
        "create_event" | "event" => {
            let calendar_id = calendar(store, fm.str("calendar"), fm.str("account"))?;
            let mut e = Event {
                id: None,
                calendar_id,
                uid: None,
                title: String::new(),
                description: String::new(),
                location: String::new(),
                start: Utc::now(),
                end: Utc::now(),
                all_day: false,
                timezone: None,
                rrule: None,
                attendees: vec![],
                organizer: None,
                status: EventStatus::Confirmed,
                my_status: None,
            };
            if fm.str("start").is_none() {
                bail!("missing `start:`");
            }
            apply_event_fields(&mut e, &fm, body)?;
            if e.title.is_empty() {
                bail!("missing `title:`");
            }
            ProposalAction::CreateEvent { event: e }
        }
        "update_event" => {
            let id = fm.str("event").ok_or_else(|| anyhow!("missing `event:` (an event_id from AGENDA.md)"))?;
            let mut e = crate::calendar::get_event(store, id)?.ok_or_else(|| anyhow!("unknown event {id}"))?;
            apply_event_fields(&mut e, &fm, body)?;
            ProposalAction::UpdateEvent { event: e }
        }
        "cancel_event" => ProposalAction::CancelEvent { event_id: fm.str("event").ok_or_else(|| anyhow!("missing `event:`"))?.to_string() },
        "respond_to_invite" | "respond" => {
            let response = match fm.str("response").unwrap_or_default().to_lowercase().as_str() {
                "accept" | "accepted" | "yes" => PartStat::Accepted,
                "decline" | "declined" | "no" => PartStat::Declined,
                "tentative" | "maybe" => PartStat::Tentative,
                other => bail!("response: expected accept, decline or tentative, got {other:?}"),
            };
            ProposalAction::RespondToInvite { event_id: fm.str("event").ok_or_else(|| anyhow!("missing `event:`"))?.to_string(), response }
        }
        "save_contact" | "contact" | "add_contact" | "update_contact" => ProposalAction::SaveContact { contact: contact(store, &fm, body)? },
        other => bail!(
            "unknown action {other:?}; use reply, draft, archive, move, labels, mark_read, mark_unread, flag, unflag, create_event, update_event, cancel_event, respond_to_invite or save_contact"
        ),
    };
    let why = fm.str("why").or(fm.str("rationale")).unwrap_or_default().to_string();
    Ok(DroppedProposal {
        agent_token: fm.str("agent_token").map(str::to_string),
        agent: fm.str("agent").map(str::to_string),
        action,
        rationale: why,
        source_thread_ids: threads,
    })
}

/// A new contact, or changes to `contact:`. Fields not given stay as they are;
/// `emails:` replaces the list and `add_emails:` appends to it.
pub(crate) fn contact(store: &Store, fm: &FrontMatter, body: String) -> Result<Contact> {
    let mut c = match fm.str("contact") {
        Some(id) => {
            let s = store.get_contact(id)?.filter(|c| !c.deleted).ok_or_else(|| anyhow!("unknown contact {id} (use a contact_id from contacts/CONTACTS.md)"))?;
            Contact { id: Some(s.id), fields: s.fields, ..Default::default() }
        }
        None => Contact::default(),
    };
    let f = &mut c.fields;
    if let Some(n) = fm.str("name") {
        f.name = n.to_string();
    }
    let email = |s: &String| -> Result<ContactEmail> { Ok(ContactEmail { email: address(s)?.email, label: None }) };
    if fm.get("emails").is_some() || fm.get("email").is_some() {
        f.emails = fm.list("emails").iter().chain(fm.list("email").iter()).map(email).collect::<Result<_>>()?;
    }
    for e in fm.list("add_emails") {
        f.emails.push(email(&e)?);
    }
    if fm.get("phones").is_some() || fm.get("phone").is_some() {
        f.phones = fm.list("phones").into_iter().chain(fm.list("phone")).map(|number| ContactPhone { number, label: None }).collect();
    }
    if let Some(o) = fm.str("organization").or(fm.str("org")).or(fm.str("company")) {
        f.organization = o.to_string();
    }
    if let Some(t) = fm.str("title") {
        f.title = t.to_string();
    }
    if let Some(b) = fm.str("birthday") {
        f.birthday = Some(b.to_string());
    }
    if !body.is_empty() {
        f.notes = body;
    }
    if c.id.is_none() && c.fields.name.trim().is_empty() && c.fields.emails.is_empty() {
        bail!("give `name:` or `emails:` for a new contact, or `contact:` to change one");
    }
    Ok(c)
}
