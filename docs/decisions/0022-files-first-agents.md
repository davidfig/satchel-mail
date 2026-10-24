# 0022. Files are the main way agents work with Satchel

- **Status:** Accepted
- **Date:** 2026-10-24
- **Supersedes:** 0007

## Context

0007 made MCP the agent interface and `~/Satchel/` a one-way export. The only
way to write through the folder is a JSON file with a pasted-in agent token. In
practice the agents people use most (Claude Code, Cursor, Codex) are best at
files: they grep, glob, read and write without setup, and pull only the files
they need into context. MCP needs per-client configuration and a token to copy
around.

The story we want to tell users is simple: *point your AI at your Satchel
folder.* It can read your mail and calendar, find patterns, and propose
changes — drafts, archiving, labels, events — that you approve in Review.

## Decision

**The AI folder is the primary agent interface.** MCP stays as a second door
for clients without file access (Claude Desktop, web chat apps) and for the
hosted version (0018), where there is no local filesystem. Both end in the same
`Command::CreateProposal`; 0006 (read and propose only) is unchanged.

**The AI folder is not the data directory.** The data directory holds
`satchel.db`, blobs and (with `SATCHEL_SECRETS=file`) `secrets.json`; write
access there bypasses every safeguard. The two stay in separate locations, and
the docs and UI talk only about the AI folder.

**Reading** works as today: `INDEX.md`, per-mailbox lists, `thread.md` +
`meta.json`, `AGENDA.md`, `.ics`. The mirror settings (accounts, history window,
raw messages, attachments) are the scope for folder agents, so "what's in the
folder is what your AI sees".

**Writing** is a Markdown file in `proposals/inbox/` with a short front matter
block and the body as the message text:

```markdown
---
action: reply
thread: 7f3a9c…
to: bob@example.com
why: Confirms the Monday slot Bob asked for
---
Monday at 10 works for me — see you then.
```

```markdown
---
action: archive
threads: [7f3a9c…, 91be02…]
why: Shipping notifications older than a month
---
```

Every `ProposalAction` gets a front matter form; JSON files keep working. The
front matter is a small subset (`key: value`, inline lists) parsed in-house —
no YAML dependency.

**No token is required.** Write access to the folder is the credential: the
worst a writer can do is add proposals the user must approve. File proposals
are attributed to a built-in "AI folder" agent whose scope follows the folder
settings, with the default rate limit; Settings → AI folder can turn file
proposals off. An optional `agent:` field is a self-declared label
shown in Review, not trusted. A file that does carry a valid `agent_token`
gets that agent's identity and policy, as now.

**Feedback goes back into the folder.** An ingested file moves to
`proposals/processed/` with a `status:` line (pending, approved, edited,
rejected) and the user's note, rewritten when the user decides, so an agent can
check what happened to its proposals by reading files.

## Consequences

- Setup for most users drops to "give your AI access to `~/Satchel`".
- Per-agent scopes only exist over MCP or with a token; all folder agents share
  the folder's scope. Different scopes for different agents would need more
  than one folder (not planned).
- Any local process that can write the folder can fill the Review queue. Rate
  limits and the existing risk warnings are the defence; a hostile local
  process already has worse options.
- The mirror has to be good enough to work from: complete within its window,
  stable paths, ids an agent can copy into a proposal.
- `docs/AI_AGENTS.md`, the mirror README and Settings copy flip to files first.
- How much an agent may do without asking (drafts, archiving, sending
  replies) is up to the user; see 0023.
