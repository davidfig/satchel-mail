//! The tool list and its JSON schemas.

use super::*;

pub(crate) fn schema(props: Value, required: &[&str]) -> JsonObject {
    let v = json!({ "type": "object", "properties": props, "required": required });
    v.as_object().cloned().unwrap_or_default()
}

pub(crate) fn ids_prop(desc: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": desc })
}

pub(crate) fn addr_prop(desc: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": desc })
}

pub(crate) fn rationale_prop() -> Value {
    json!({ "type": "string", "description": "Why you are proposing this, shown to the user." })
}

pub(crate) fn tools() -> Vec<Tool> {
    let ro = |t: Tool| t.annotate(ToolAnnotations::new().read_only(true));
    let src = ids_prop("Thread ids this proposal is based on.");
    vec![
        ro(Tool::new("whoami", "Show this agent's name, its access policy, and server info.", schema(json!({}), &[]))),
        ro(Tool::new("list_accounts", "List the mail accounts you can read.", schema(json!({}), &[]))),
        ro(Tool::new("list_mailboxes", "List folders/labels with unread and total counts.", schema(json!({ "account_id": { "type": "string" } }), &[]))),
        ro(Tool::new(
            "search_threads",
            "Search or list conversations, newest first. `query` supports free text and operators: from:, to:, with: (sender or recipient), subject:, has:attachment, is:unread, is:flagged, before:YYYY-MM-DD, after:YYYY-MM-DD, in:<mailbox>. from:, to: and with: take a comma-separated list; a full address matches exactly. Use role (inbox, sent, drafts, archive, trash, junk, flagged) or mailbox_id to browse.",
            schema(json!({
                "query": { "type": "string" },
                "mailbox_id": { "type": "string" },
                "role": { "type": "string", "enum": ["inbox", "sent", "drafts", "archive", "trash", "junk", "flagged", "all", "important"] },
                "account_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                "before": { "type": "string", "description": "Paging: the last_date of the last thread you received (RFC 3339)." },
                "before_thread_id": { "type": "string", "description": "Paging: the thread_id of that last thread (avoids skipping threads with the same date)." }
            }), &[]),
        )),
        ro(Tool::new("get_thread", "Read a whole conversation as plain text, oldest first, quoted history removed.", schema(json!({ "thread_id": { "type": "string" } }), &["thread_id"]))),
        ro(Tool::new("get_message", "Read one message (plain text body, headers, attachment list).", schema(json!({ "message_id": { "type": "string" } }), &["message_id"]))),
        ro(Tool::new("get_attachment", "Read an attachment, if the user allowed attachments for this agent. Text attachments are returned as text.", schema(json!({ "message_id": { "type": "string" }, "attachment_id": { "type": "string" } }), &["message_id", "attachment_id"]))),
        ro(Tool::new("list_calendars", "List calendars.", schema(json!({}), &[]))),
        ro(Tool::new("list_events", "List event occurrences in a time range (recurrences expanded). Dates are RFC 3339.", schema(json!({ "from": { "type": "string" }, "to": { "type": "string" }, "calendar_ids": ids_prop("Limit to these calendars.") }), &["from", "to"]))),
        ro(Tool::new("get_event", "Get one event (the series master for recurring events).", schema(json!({ "event_id": { "type": "string" } }), &["event_id"]))),
        ro(Tool::new("find_free_time", "Find free slots on weekdays within working hours.", schema(json!({
            "from": { "type": "string" }, "to": { "type": "string" },
            "duration_minutes": { "type": "integer", "minimum": 5 },
            "day_start_hour": { "type": "integer" }, "day_end_hour": { "type": "integer" },
            "timezone": { "type": "string", "description": "IANA zone, e.g. America/New_York" }
        }), &["from", "to", "duration_minutes"]))),
        Tool::new(
            "propose_reply",
            "Propose a reply to a message. The reply is addressed, subject-prefixed and quoted automatically; you supply only the new text. The user reviews it and sends it themselves.",
            schema(json!({
                "message_id": { "type": "string" },
                "body_text": { "type": "string", "description": "Your reply text (plain text)." },
                "reply_all": { "type": "boolean" },
                "cc": addr_prop("Extra Cc addresses."),
                "extra_to": addr_prop("Extra To addresses."),
                "rationale": rationale_prop()
            }), &["message_id", "body_text", "rationale"]),
        ),
        Tool::new("propose_draft", "Propose a new message (not a reply).", schema(json!({
            "account_id": { "type": "string" }, "to": addr_prop("Recipients."), "cc": addr_prop("Cc."),
            "subject": { "type": "string" }, "body_text": { "type": "string" },
            "source_thread_ids": src, "rationale": rationale_prop()
        }), &["account_id", "to", "subject", "body_text", "rationale"])),
        Tool::new("propose_archive", "Propose archiving messages (removes them from the inbox).", schema(json!({ "message_ids": ids_prop("Messages to archive."), "source_thread_ids": src, "rationale": rationale_prop() }), &["message_ids", "rationale"])),
        Tool::new("propose_move", "Propose moving messages to a folder.", schema(json!({ "message_ids": ids_prop("Messages."), "mailbox_id": { "type": "string" }, "source_thread_ids": src, "rationale": rationale_prop() }), &["message_ids", "mailbox_id", "rationale"])),
        Tool::new("propose_labels", "Propose adding/removing labels (Gmail-style accounts).", schema(json!({ "message_ids": ids_prop("Messages."), "add": ids_prop("Mailbox ids to add."), "remove": ids_prop("Mailbox ids to remove."), "source_thread_ids": src, "rationale": rationale_prop() }), &["message_ids", "rationale"])),
        Tool::new("propose_mark_read", "Propose marking messages read (or unread with read=false).", schema(json!({ "message_ids": ids_prop("Messages."), "read": { "type": "boolean" }, "source_thread_ids": src, "rationale": rationale_prop() }), &["message_ids", "rationale"])),
        Tool::new("propose_flag", "Propose flagging/starring messages (or unflagging with flagged=false).", schema(json!({ "message_ids": ids_prop("Messages."), "flagged": { "type": "boolean" }, "source_thread_ids": src, "rationale": rationale_prop() }), &["message_ids", "rationale"])),
        Tool::new("propose_event", "Propose a new calendar event. Dates are RFC 3339; set timezone for correct recurrence/DST.", schema(json!({
            "calendar_id": { "type": "string" }, "title": { "type": "string" },
            "start": { "type": "string" }, "end": { "type": "string" },
            "description": { "type": "string" }, "location": { "type": "string" },
            "all_day": { "type": "boolean" }, "timezone": { "type": "string" },
            "rrule": { "type": "string", "description": "e.g. FREQ=WEEKLY;BYDAY=MO" },
            "attendees": addr_prop("Invitees (they receive invitations once the user approves)."),
            "source_thread_ids": src, "rationale": rationale_prop()
        }), &["calendar_id", "title", "start", "end", "rationale"])),
        Tool::new("propose_event_change", "Propose changing an existing event. Omit fields you don't want to change.", schema(json!({
            "event_id": { "type": "string" }, "title": { "type": "string" }, "start": { "type": "string" }, "end": { "type": "string" },
            "description": { "type": "string" }, "location": { "type": "string" }, "timezone": { "type": "string" },
            "source_thread_ids": src, "rationale": rationale_prop()
        }), &["event_id", "rationale"])),
        Tool::new("propose_cancel_event", "Propose cancelling (deleting) an event.", schema(json!({ "event_id": { "type": "string" }, "source_thread_ids": src, "rationale": rationale_prop() }), &["event_id", "rationale"])),
        Tool::new("propose_invite_response", "Propose accepting, declining or tentatively accepting an invitation.", schema(json!({ "event_id": { "type": "string" }, "response": { "type": "string", "enum": ["accept", "decline", "tentative"] }, "source_thread_ids": src, "rationale": rationale_prop() }), &["event_id", "response", "rationale"])),
        ro(Tool::new("search_contacts", "Search the user's contacts by name, address, phone or organization (omit query to list them).", schema(json!({ "query": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 } }), &[]))),
        ro(Tool::new("get_contact", "Get one contact with all its addresses.", schema(json!({ "contact_id": { "type": "string" } }), &["contact_id"]))),
        Tool::new("propose_contact", "Propose a new contact (no contact_id) or a change to one. Omit fields you don't want to change; `emails` replaces the list, `add_emails` appends. Contacts are never deleted by agents; use propose_merge_contacts for duplicates.", schema(json!({
            "contact_id": { "type": "string" }, "name": { "type": "string" },
            "emails": addr_prop("All addresses (replaces the list)."), "add_emails": addr_prop("Addresses to add."),
            "phones": ids_prop("Phone numbers (replaces the list)."),
            "organization": { "type": "string" }, "title": { "type": "string" },
            "birthday": { "type": "string", "description": "YYYY-MM-DD, or --MM-DD without a year" },
            "notes": { "type": "string" },
            "source_thread_ids": src, "rationale": rationale_prop()
        }), &["rationale"])),
        Tool::new("propose_merge_contacts", "Propose merging duplicate contacts for one person. The first contact is kept and the others are merged into it. A merge can't be undone from Satchel, so it always waits for the user's review.", schema(json!({
            "contact_ids": ids_prop("Two or more contact ids; the first is kept."),
            "source_thread_ids": src, "rationale": rationale_prop()
        }), &["contact_ids", "rationale"])),
        ro(Tool::new("get_proposal_status", "Check whether the user approved, edited, or rejected a proposal (with their note).", schema(json!({ "proposal_id": { "type": "string" } }), &["proposal_id"]))),
        ro(Tool::new("list_my_proposals", "List proposals you made, optionally filtered by status.", schema(json!({ "status": { "type": "string", "enum": ["pending", "applied", "rejected", "stale", "failed"] } }), &[]))),
    ]
}
