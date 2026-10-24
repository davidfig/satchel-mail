# 0023. Users choose how much agents may do on their own

- **Status:** Accepted
- **Date:** 2026-10-24
- **Amends:** 0006

## Context

0006 sends every agent change through Review. That suits people who want to
check every email, but it's tedious for people who'd happily let an agent
archive newsletters or save drafts without asking. 0006 left "policy-based
auto-approve" as a later decision; this is that decision.

The risk that 0006 guards against hasn't changed: mail is written by
strangers, and an agent reading it can be prompt-injected. The worst outcomes
are mail sent to new people (data leaving), mail sent in your name to the
wrong person, and things lost.

## Decision

**Agents still only propose.** The agent-facing surface stays read +
`CreateProposal` (`Command::allowed_for_agent`). What changes is what happens
to a proposal: the user's policy can approve it on arrival. The policy acts,
not the agent, and agents can never read-write their own policy (settings
commands stay off-limits to them).

**A level for each kind of action**, set per agent (the AI folder agent from
0022 has one too):

| Action | Levels |
|---|---|
| Archive, move, labels, mark read, flag | Ask · Do it |
| Drafts | Ask · Save to Drafts · Send replies |
| Calendar: create, update, respond, cancel | Ask · Do it |

Each also has **Off**, which rejects the proposal outright. Settings offer
presets — **Review everything** (default), **Tidy up for me** (mail
organising and drafts done automatically, sending and calendar asked),
**Hands off** (everything up to sending replies) — with the table underneath
for anyone who wants to adjust it.

**Risky proposals wait unless the user says otherwise.** By default a
proposal with any risk warning waits in Review, whatever the level:
recipients outside the conversation, new conversations, Bcc, forwarded
attachments, outgoing invitations, moves to Trash, cancellations, and batches
over a size limit (e.g. 50 messages). This covers the data-leak and
wrong-recipient cases, so by default "Send replies" only means replying to
people already in the thread.

Users can turn this off per agent ("Let risky changes through without
asking"). It's off by default and never part of a preset. Turning it on shows
a strong caution and needs an explicit confirmation, along the lines of:

> An agent that reads a malicious email could be tricked into sending your
> mail or files to strangers, or inviting people in your name, and Satchel
> won't stop it. Only turn this on if you trust everything this agent reads.

With it on, the warnings still apply: risky changes are highlighted in
Activity, and risky sends get a longer undo window than ordinary ones.

**Hard limits no setting removes:** nothing permanently deletes, changes
settings, or creates server-side rules or forwarding, at any level.

**Auto-applied changes are visible and reversible.** They go through the ops
queue (0008) like any user change, show in Activity as "done by <agent>", and
reversible ones get Undo. Automatic sends wait in the outbox for an undo
window (default 5 minutes) before going out. A **Pause agents** switch drops
every agent to Ask at once.

**Agents can see their own levels** (in `whoami` and the AI folder README), so
they know which proposals will apply and which will wait.

## Consequences

- One codepath for users: Review stays the place for anything that waits, and
  Activity becomes the place to see what agents did.
- The approval logic moves from "user clicked approve" to "user or policy
  approved"; the audit log records which.
- Sending without a click is new. By default the warnings rule and the undo
  window are what make it acceptable; changes to the warning list now matter
  for safety and need care.
- Users who turn off the warnings rule accept the prompt-injection risk
  themselves. The product's guarantee shrinks to the hard limits, the undo
  window and the Activity log, and the docs must say so plainly.
- `docs/AI_AGENTS.md`, Settings and the propose-only wording in 0006 and
  `CLAUDE.md` need updating to say "agents propose; your policy decides".
