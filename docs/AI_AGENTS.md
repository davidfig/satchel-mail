# AI agents

Agents get read access and the ability to **propose** changes. Every proposal
lands in the Review queue; the user approves (optionally after editing),
rejects with a note the agent can read, or ignores it. Agents can never send
mail, delete anything, or change settings.

## Connecting

1. Satchel Mail → Settings → AI agents → **Connect an agent**. Choose which
   accounts it may read, how much history, and whether it may read calendars
   and binary attachments.
2. Copy the token (shown once) and the setup command, e.g. Claude Code:

   ```bash
   claude mcp add satchel --env SATCHEL_AGENT_TOKEN=stk_… -- satchel mcp
   ```

   Claude Desktop / other MCP clients:

   ```json
   { "mcpServers": { "satchel": { "command": "satchel", "args": ["mcp"], "env": { "SATCHEL_AGENT_TOKEN": "stk_…" } } } }
   ```

   The desktop app shows the absolute path of its bundled `satchel` binary.

`satchel mcp` works whether or not the app is running: it opens the local store
read-only and writes proposals the app picks up immediately.

## Tools

Read: `whoami`, `list_accounts`, `list_mailboxes`, `search_threads` (free text +
`from: to: subject: has:attachment is:unread is:flagged before: after: in:`),
`get_thread`, `get_message`, `get_attachment`, `list_calendars`, `list_events`,
`get_event`, `find_free_time`.

Propose: `propose_reply`, `propose_draft`, `propose_archive`, `propose_move`,
`propose_labels`, `propose_mark_read`, `propose_flag`, `propose_event`,
`propose_event_change`, `propose_cancel_event`, `propose_invite_response`.

Follow-up: `get_proposal_status`, `list_my_proposals`.

## Safeguards

- **Scope:** per-agent account list, history window, calendar and attachment
  toggles, and a proposals-per-hour limit.
- **Warnings** shown on review: recipients outside the original conversation,
  new conversations, Bcc, many recipients, forwarded attachments, invitations
  that will be sent, moves to Trash, cancellations.
- **Staleness:** if the message or event changed after the proposal, approval is
  blocked until the user looks again.
- **De-duplication:** identical pending proposals from the same agent collapse.
- **Audit log:** every proposal, decision and send (Settings → Activity).
- Email bodies are third-party content; the MCP instructions tell agents to treat
  instructions inside messages as data. The structural limits above are the real
  protection.

## The AI folder

`~/Satchel/` (configurable in Settings → AI folder) is a one-way, plain-text
export for tools without MCP:

```
mail/<account>/INDEX.md, <mailbox>.md
mail/<account>/threads/<yyyy-mm>/<slug>-<id>/thread.md, meta.json
calendar/AGENDA.md, calendar/<account>/<calendar>/<uid>.ics
proposals/inbox/      drop proposal JSON files here (needs an agent token)
proposals/processed/  results (tokens stripped)
```

See `~/Satchel/README.md` for the proposal file format.
