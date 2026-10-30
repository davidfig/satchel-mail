# 0006. AI agents can only read and propose

- **Status:** Accepted, amended by 0023 and 0044
- **Date:** 2026-10-23

## Context

Email content is attacker-controlled; an agent reading mail can be
prompt-injected. The product promise is that nothing happens without the user.

## Decision

Agents (any caller authenticated with an agent token) may only run read
commands and `CreateProposal` (`Command::allowed_for_agent`). Proposals land in
the Review queue; the user approves (optionally after editing), rejects with a
note, or ignores them. Approving a draft opens it in the composer — the user
presses Send. There is no tool for sending, deleting or changing settings.

Safeguards: per-agent policy (accounts, history window, calendar, binary
attachments, rate limit), risk warnings (outside recipients, Bcc, forwarded
attachments, invitations, Trash moves), staleness checks on approval,
de-duplication of identical pending proposals, and an audit log.

## Consequences

- Limits blast radius structurally rather than relying on model behaviour.
- Agents can't automate low-risk actions yet; policy-based auto-approve is a
  possible later decision.

**Amended by 0023:** agents still only propose, but the user's per-agent
policy may approve a proposal on arrival, up to sending replies.

**Amended by 0044:** approving a draft can send it straight from Review
(the user still presses Send); "Save as draft" keeps the composer route.
