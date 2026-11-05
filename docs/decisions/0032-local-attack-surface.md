# 0032. Local attackers: the web session and AI-folder symlinks

- **Status:** Accepted; extended by 0040
- **Date:** 2026-10-24

## Context

Two ways around the rule that agents only propose (0006) came from programs
on the same computer, not from email.

- `satchel serve` set its user session cookie on every page it served. Any
  local program that could make an HTTP request got full user access:
  sending, settings, its own agent policy. SameSite doesn't separate ports,
  so a page served on another localhost port could also post to `/api/rpc`
  with the cookie (a `text/plain` body needs no preflight) and read events
  from the WebSocket.
- Satchel followed symlinks in `~/Satchel/`. An agent could link an inbox
  file to `~/.ssh/id_ed25519` and read it back from `processed/`, or plant a
  dangling link where Satchel would write and create any file it liked.

## Decision

- **Session:** the server prints a link carrying the session token. Opening
  it sets the cookie and redirects to a clean URL; nothing else hands the
  cookie out. The token is kept 0600 in the data directory, so tabs survive
  a restart. User calls must carry the session (cookie or bearer). A browser
  call must also come from our own origin (`Origin`, `Sec-Fetch-Site`), and
  RPC requires `application/json`. `SATCHEL_DEV_ORIGIN` allows the Vite dev
  server.
- **AI folder:** reads of agent-reachable files refuse symlinks. The check
  compares the opened file to the path, so a link swapped in meanwhile is
  caught too. Writes go to a fresh temp file renamed over the target, which
  replaces a planted link instead of following it. A symlinked directory
  under the root is refused. The root itself may be a symlink, since the
  user chose it. The folder is 0700.

## Consequences

- The web UI can't be opened from a bookmark of the bare address on a new
  browser; use the printed link once.
- Anything with access to the data directory can still act as the user. That
  directory holds the mailbox itself, so it has always been trusted.
