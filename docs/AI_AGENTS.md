# AI agents

Give your AI access to your Satchel folder (`~/Satchel/`) and it can read your
mail and calendars, find patterns, and propose changes — drafts, archiving,
labels, events. Agents only ever *propose*; how much happens without asking is
up to you ([0022](decisions/0022-files-first-agents.md),
[0023](decisions/0023-agent-autonomy-levels.md)).

Proposals you review land in the Review queue: approve (optionally after
editing), reject with a note the agent can read, or ignore. Agents can never
delete anything permanently, change settings, or create server-side rules.

> **Status:** the folder (Markdown or JSON proposals, no token needed) and MCP
> work today, with every proposal reviewed by hand. Autonomy levels are
> accepted but not built yet; they're marked *planned* below.

## The AI folder

Turn it on in Settings → AI folder (default `~/Satchel/`), choose which
accounts and how much history to include, then point your agent at it — for
Claude Code, open it as a working directory or add it with `--add-dir`.

To keep it somewhere else, use **Choose…** next to Location. Saving a new
location moves the folder, with any proposals waiting in it; the new place
must be empty. `~/Satchel/` is a good default: macOS asks before apps (and AI
tools) can read Documents, Desktop or Downloads, but not a folder in your home.

Give agents the AI folder, **not** Satchel's data directory. The data
directory holds the database and, on headless setups, credentials; write
access there gets around every safeguard.

```
README.md                                  how to read and propose (for the agent)
sync.json                                  last sync per account; complete: false mid-rewrite
mail/<account>/INDEX.md, <mailbox>.md      newest threads first (empty mailboxes left out)
mail/<account>/index.jsonl                 the same rows as INDEX.md, for tools
mail/<account>/threads/<yyyy-mm>/<slug>-<id>/thread.md, meta.json
calendar/AGENDA.md, calendar/<account>/<calendar>/<uid>.ics
contacts/CONTACTS.md                       the address book, with contact ids
contacts/people/<name>-<id>.md             one person; edit or add one to propose a change
proposals/inbox/                           drop proposal files here
proposals/processed/                       results (shared; not authoritative)
```

What's in the folder is what your AI sees: the folder's settings are its scope.

### Proposing changes

An agent writes a Markdown file to `proposals/inbox/`:

```markdown
---
action: reply
thread: 7f3a9c…
why: Confirms the Monday slot Bob asked for
agent: Claude
---
Monday at 10 works for me — see you then.
```

Actions: `reply`, `draft`, `archive`, `move`, `labels`, `mark_read`,
`mark_unread`, `flag`, `unflag`, `create_event`, `update_event`,
`cancel_event`, `respond_to_invite`, `save_contact`. `~/Satchel/README.md`
lists each one's fields; agents should read it first.

Contacts can also be changed by editing a person's file in
`contacts/people/`, or added by creating one. Satchel turns the edit into a
`save_contact` proposal with only the fields the agent changed, and moves
the file to `proposals/processed/` like an inbox file (0029).

Every file comes from the built-in **AI folder** agent: it sees what the
folder holds and is limited to 60 proposals an hour. `agent:` is a
self-declared name shown in Review, not proof of who wrote the file. Turn
folder proposals off in Settings → AI folder. Files carrying an agent token
are refused: any tool with folder access could read the token and borrow
that agent's wider limits, so connected agents use MCP or the HTTP endpoint
(0042). JSON files in the API's proposal shape also work.

Once read, the file moves to `proposals/processed/` with `status:` (pending,
approved, edited, rejected, stale, failed or error), the proposal id, any
warnings, and the status is updated when you decide. Your review note isn't
written there: every agent can read and edit `processed/`, so nothing in it is
proof of what you said.

## MCP

For clients without file access (Claude Desktop, web chat apps) and, later,
the hosted version. Settings → AI agents → **Connect an agent** creates a token
and shows the setup command, e.g. Claude Code:

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

Read: `whoami`, `list_accounts`, `list_mailboxes`, `search_threads` (free text +
`from: to: with: subject: has:attachment is:unread is:flagged before: after: in:`),
`get_thread`, `get_message`, `get_attachment`, `list_calendars`, `list_events`,
`get_event`, `find_free_time`, `search_contacts`, `get_contact`.

Propose: `propose_reply`, `propose_draft`, `propose_archive`, `propose_move`,
`propose_labels`, `propose_mark_read`, `propose_flag`, `propose_event`,
`propose_event_change`, `propose_cancel_event`, `propose_invite_response`,
`propose_contact` (create or change a contact; adding an address to someone
or renaming them is flagged as a risk in Review), `propose_merge_contacts`
(the first contact is kept; always flagged as a risk, decision 0045). Agents
never delete contacts.

Follow-up: `get_proposal_status`, `list_my_proposals`.

## How much agents may do (*planned*)

Each agent gets a level per kind of action:

| Action | Levels |
|---|---|
| Archive, move, labels, mark read, flag | Off · Ask · Do it |
| Drafts | Off · Ask · Save to Drafts · Send replies |
| Calendar: create, update, respond, cancel | Off · Ask · Do it |

Presets: **Review everything** (default), **Tidy up for me**, **Hands off**.
Changes made without asking show in Activity with Undo where possible;
automatic sends wait in the outbox for an undo window. **Pause agents** puts
everything back to Ask.

By default a proposal with a risk warning always waits for you, whatever the
level. You can turn that off per agent ("Let risky changes through without
asking") after a strong caution: an agent that reads a malicious email could
then send your mail to strangers and Satchel won't stop it.

## Safeguards

- **Scope:** the AI folder's settings; for token agents, a per-agent account
  list, history window, calendar, contacts and attachment toggles, and a
  proposals-per-hour limit.
- **Warnings:** recipients outside the original conversation, new
  conversations, Bcc, many recipients, forwarded attachments, invitations that
  will be sent, moves to Trash, cancellations.
- **Staleness:** if the message or event changed after the proposal, approval is
  blocked until you look again.
- **De-duplication:** identical pending proposals from the same agent collapse.
- **Audit log:** every proposal, decision and send (Settings → Activity).
- **Hard limits:** no permanent deletes, settings changes, or server-side rules
  or forwarding, at any level.
- Email bodies are third-party content; the folder README and MCP instructions
  tell agents to treat instructions inside messages as data. The structural
  limits above are the real protection.
