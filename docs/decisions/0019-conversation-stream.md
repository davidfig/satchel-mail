# 0019. Conversations as a chat-style stream

- **Status:** Accepted
- **Date:** 2026-10-23

## Context
The reader stacked every message as a full card, like Apple Mail. Long and
group threads were hard to follow: quoted history repeated in every message,
nothing marked where you left off, it was unclear who was answering whom, and
people joining or leaving the thread were buried in headers. Replying took a
button and a separate composer.

## Decision
The reader renders a thread as a stream (`ui/src/views/conversation.ts`):

- Person-to-person mail becomes bubbles, yours on the right, showing only what
  each person wrote; quoted history is folded behind "•••". Laid-out mail
  (tables, several images, HTML-only) stays a full-width card in the stream.
- Day separators, a "New" line before the first unread message, and a fold
  ("N earlier messages") between the first message and the recent ones.
- A reply to anything other than the previous message shows a quote of its
  parent (from `In-Reply-To`); clicking it jumps there.
- Recipient changes between messages appear as lines ("Bob added Dan").
- "You" is the account the thread belongs to, not every local account.
- An always-open reply box replaces the Reply / Reply all buttons. When an
  external agent has a pending draft proposal for the thread, the box shows it
  for editing; sending approves the proposal. Satchel itself has no AI
  features (see 0006, 0007).
- Later amendment: in a thread you haven't written in, the box starts as a
  "Reply" button when the core marks the latest message `bulk`: List-*,
  `Precedence: bulk|list|junk` or `Auto-Submitted` headers, or a machine
  reply address (no-reply, tokenized `reply-…@`). A helpdesk "reply above
  this line" marker in the body overrides all of these, since the sender is
  asking for an answer (Zendesk sets `Auto-Submitted` on ticket replies).
- The HTML sanitizer keeps `div id` and `blockquote type` so Outlook and Apple
  Mail quote markers survive for folding.

## Consequences
Threads read top to bottom with much less repetition. Quote detection is
heuristic: unusual clients may fold too much or too little, and "Show original
formatting" / "•••" are the escape hatches. Inline replies (answers written
between quoted lines) are hidden in the fold.
