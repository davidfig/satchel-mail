//! Running one tool call as the agent: arguments to a Command.

use super::*;

impl SatchelMcp {
    pub(crate) async fn call(&self, name: &str, a: &Value) -> Result<Value, String> {
        let s = |k: &str| a.get(k).and_then(Value::as_str).map(|x| x.to_string());
        let req = |k: &str| s(k).ok_or_else(|| format!("missing required argument `{k}`"));
        let ids = |k: &str| -> Result<Vec<String>, String> {
            serde_json::from_value(a.get(k).cloned().unwrap_or(json!([]))).map_err(|_| format!("`{k}` must be an array of ids"))
        };
        let addrs = |k: &str| -> Result<Vec<Address>, String> {
            let v = a.get(k).cloned().unwrap_or(json!([]));
            let arr = v.as_array().cloned().unwrap_or_default();
            arr.into_iter()
                .map(|x| match x {
                    Value::String(e) => Ok(Address::new(e)),
                    o => serde_json::from_value(o).map_err(|_| format!("`{k}` must contain email strings or {{email, name}} objects")),
                })
                .collect()
        };
        let date = |k: &str| -> Result<chrono::DateTime<chrono::Utc>, String> {
            let v = req(k)?;
            chrono::DateTime::parse_from_rfc3339(&v).map(|d| d.with_timezone(&chrono::Utc)).map_err(|_| format!("`{k}` must be an RFC 3339 date-time"))
        };
        let rationale = s("rationale").unwrap_or_default();
        let propose = |action: ProposalAction, sources: Vec<String>| Command::CreateProposal { action, rationale: rationale.clone(), source_thread_ids: sources };

        match name {
            "whoami" => {
                let caller = self.caller()?;
                let Caller::Agent(agent) = caller else { unreachable!() };
                let info = self.run(Command::Info).await?;
                Ok(json!({ "agent": agent.name, "policy": agent.policy, "server": info }))
            }
            "list_accounts" => {
                let v = self.run(Command::ListAccounts).await?;
                let accounts: Vec<Account> = serde_json::from_value(v).unwrap_or_default();
                Ok(json!(accounts.iter().map(|a| json!({ "account_id": a.id, "email": a.email, "name": a.display_name, "kind": a.kind })).collect::<Vec<_>>()))
            }
            "list_mailboxes" => self.run(Command::ListMailboxes { account_id: s("account_id") }).await,
            "search_threads" => {
                let role = s("role").map(|r| serde_json::from_value::<MailboxRole>(json!(r))).transpose().map_err(|_| "unknown role")?;
                let before = match s("before") {
                    Some(b) => Some(chrono::DateTime::parse_from_rfc3339(&b).map_err(|_| "`before` must be RFC 3339")?.with_timezone(&chrono::Utc)),
                    None => None,
                };
                let q = ThreadQuery {
                    mailbox_id: s("mailbox_id"),
                    role,
                    account_id: s("account_id"),
                    search: s("query"),
                    limit: a.get("limit").and_then(Value::as_u64).map(|n| n.min(200) as u32).or(Some(25)),
                    before,
                    before_id: s("before_thread_id"),
                    offset: None,
                };
                self.run(Command::ListThreads { query: q }).await
            }
            "get_thread" => {
                let v = self.run(Command::GetThread { thread_id: req("thread_id")? }).await?;
                Ok(slim_thread(v))
            }
            "get_message" => {
                let v = self.run(Command::GetMessage { message_id: req("message_id")? }).await?;
                Ok(slim_message(v))
            }
            "get_attachment" => self.run(Command::GetAttachment { message_id: req("message_id")?, attachment_id: req("attachment_id")? }).await,
            "list_calendars" => self.run(Command::ListCalendars).await,
            "list_events" => self.run(Command::ListEvents { from: date("from")?, to: date("to")?, calendar_ids: a.get("calendar_ids").map(|_| ids("calendar_ids")).transpose()? }).await,
            "get_event" => self.run(Command::GetEvent { event_id: req("event_id")? }).await,
            "find_free_time" => {
                self.run(Command::FindFreeTime {
                    from: date("from")?,
                    to: date("to")?,
                    duration_minutes: a.get("duration_minutes").and_then(Value::as_u64).unwrap_or(30) as u32,
                    day_start_hour: a.get("day_start_hour").and_then(Value::as_u64).map(|x| x as u32),
                    day_end_hour: a.get("day_end_hour").and_then(Value::as_u64).map(|x| x as u32),
                    timezone: s("timezone"),
                })
                .await
            }
            "propose_reply" => {
                let message_id = req("message_id")?;
                let body = req("body_text")?;
                // Validate access under the agent's policy before building the template.
                let msg = self.run(Command::GetMessage { message_id: message_id.clone() }).await?;
                let thread_id = msg.get("thread_id").and_then(Value::as_str).unwrap_or_default().to_string();
                let mode = if a.get("reply_all").and_then(Value::as_bool).unwrap_or(false) { ReplyMode::ReplyAll } else { ReplyMode::Reply };
                let tmpl = self.engine.handle(&Caller::User, Command::ComposeReply { message_id, mode }).await.map_err(|e| e.message)?;
                let mut draft: Draft = serde_json::from_value(tmpl).map_err(|e| e.to_string())?;
                draft.body_text = format!("{}{}", body.trim_end(), draft.body_text);
                draft.cc.extend(addrs("cc")?);
                draft.to.extend(addrs("extra_to")?);
                self.run(propose(ProposalAction::Draft { draft }, vec![thread_id])).await
            }
            "propose_draft" => {
                let draft = Draft {
                    id: None,
                    account_id: req("account_id")?,
                    to: addrs("to")?,
                    cc: addrs("cc")?,
                    bcc: vec![],
                    subject: req("subject")?,
                    body_text: req("body_text")?,
                    body_html: None,
                    in_reply_to: None,
                    forward_of: None,
                    attachments: vec![],
                };
                self.run(propose(ProposalAction::Draft { draft }, ids("source_thread_ids")?)).await
            }
            "propose_archive" => self.run(propose(ProposalAction::Archive { message_ids: ids("message_ids")? }, ids("source_thread_ids")?)).await,
            "propose_move" => self.run(propose(ProposalAction::Move { message_ids: ids("message_ids")?, mailbox_id: req("mailbox_id")? }, ids("source_thread_ids")?)).await,
            "propose_labels" => self.run(propose(ProposalAction::Labels { message_ids: ids("message_ids")?, add: ids("add")?, remove: ids("remove")? }, ids("source_thread_ids")?)).await,
            "propose_mark_read" => {
                let seen = a.get("read").and_then(Value::as_bool).unwrap_or(true);
                self.run(propose(ProposalAction::MarkRead { message_ids: ids("message_ids")?, seen }, ids("source_thread_ids")?)).await
            }
            "propose_flag" => {
                let flagged = a.get("flagged").and_then(Value::as_bool).unwrap_or(true);
                self.run(propose(ProposalAction::Flag { message_ids: ids("message_ids")?, flagged }, ids("source_thread_ids")?)).await
            }
            "propose_event" => {
                let attendees = addrs("attendees")?
                    .into_iter()
                    .map(|x| Attendee { email: x.email, name: x.name, status: PartStat::NeedsAction, optional: false })
                    .collect();
                let event = Event {
                    id: None,
                    calendar_id: req("calendar_id")?,
                    uid: None,
                    title: req("title")?,
                    description: s("description").unwrap_or_default(),
                    location: s("location").unwrap_or_default(),
                    start: date("start")?,
                    end: date("end")?,
                    all_day: a.get("all_day").and_then(Value::as_bool).unwrap_or(false),
                    timezone: s("timezone"),
                    rrule: s("rrule"),
                    attendees,
                    organizer: None,
                    status: EventStatus::Confirmed,
                    my_status: None,
                };
                self.run(propose(ProposalAction::CreateEvent { event }, ids("source_thread_ids")?)).await
            }
            "propose_event_change" => {
                let event_id = req("event_id")?;
                let cur = self.run(Command::GetEvent { event_id }).await?;
                let mut event: Event = serde_json::from_value(cur).map_err(|e| e.to_string())?;
                if let Some(t) = s("title") {
                    event.title = t;
                }
                if let Some(d) = s("description") {
                    event.description = d;
                }
                if let Some(l) = s("location") {
                    event.location = l;
                }
                if a.get("start").is_some() {
                    event.start = date("start")?;
                }
                if a.get("end").is_some() {
                    event.end = date("end")?;
                }
                if let Some(tz) = s("timezone") {
                    event.timezone = Some(tz);
                }
                self.run(propose(ProposalAction::UpdateEvent { event }, ids("source_thread_ids")?)).await
            }
            "propose_cancel_event" => self.run(propose(ProposalAction::CancelEvent { event_id: req("event_id")? }, ids("source_thread_ids")?)).await,
            "propose_invite_response" => {
                let response = match req("response")?.to_lowercase().as_str() {
                    "accept" | "accepted" | "yes" => PartStat::Accepted,
                    "decline" | "declined" | "no" => PartStat::Declined,
                    "tentative" | "maybe" => PartStat::Tentative,
                    _ => return Err("`response` must be accept, decline or tentative".into()),
                };
                self.run(propose(ProposalAction::RespondToInvite { event_id: req("event_id")?, response }, ids("source_thread_ids")?)).await
            }
            "search_contacts" => {
                let limit = a.get("limit").and_then(Value::as_u64).map(|n| n.min(500) as u32).or(Some(50));
                self.run(Command::ListContacts { search: s("query"), limit }).await
            }
            "get_contact" => self.run(Command::GetContact { contact_id: req("contact_id")? }).await,
            "propose_contact" => {
                let mut contact = match s("contact_id") {
                    Some(id) => serde_json::from_value::<Contact>(self.run(Command::GetContact { contact_id: id }).await?).map_err(|e| e.to_string())?,
                    None => Contact::default(),
                };
                let f = &mut contact.fields;
                if let Some(n) = s("name") {
                    f.name = n;
                }
                if a.get("emails").is_some() {
                    f.emails = addrs("emails")?.into_iter().map(|x| ContactEmail { email: x.email, label: None }).collect();
                }
                f.emails.extend(addrs("add_emails")?.into_iter().map(|x| ContactEmail { email: x.email, label: None }));
                if a.get("phones").is_some() {
                    f.phones = ids("phones")?.into_iter().map(|number| ContactPhone { number, label: None }).collect();
                }
                if let Some(o) = s("organization") {
                    f.organization = o;
                }
                if let Some(t) = s("title") {
                    f.title = t;
                }
                if let Some(b) = s("birthday") {
                    f.birthday = Some(b);
                }
                if let Some(n) = s("notes") {
                    f.notes = n;
                }
                self.run(propose(ProposalAction::SaveContact { contact }, ids("source_thread_ids")?)).await
            }
            "propose_merge_contacts" => self.run(propose(ProposalAction::MergeContacts { contact_ids: ids("contact_ids")? }, ids("source_thread_ids")?)).await,
            "get_proposal_status" => {
                let v = self.run(Command::GetProposal { proposal_id: req("proposal_id")? }).await?;
                Ok(json!({ "status": v["status"], "summary": v["summary"], "review_note": v["review_note"], "decided_at": v["decided_at"] }))
            }
            "list_my_proposals" => {
                let status = s("status").map(|x| serde_json::from_value::<ProposalStatus>(json!(x))).transpose().map_err(|_| "unknown status")?;
                self.run(Command::ListProposals { status }).await
            }
            other => Err(format!("unknown tool {other}")),
        }
    }
}
