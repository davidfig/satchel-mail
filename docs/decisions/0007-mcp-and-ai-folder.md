# 0007. MCP over stdio as the agent interface, plus an AI folder

- **Status:** Superseded by 0022
- **Date:** 2026-10-23

## Context

Agents need a standard way in. Some tools speak MCP; others only read files.

## Decision

`satchel mcp` serves MCP over stdio with read and propose tools; it opens the
store in agent mode, so it works whether or not the app is running. The engine
also writes a one-way plain-text mirror (`~/Satchel/`: thread.md, meta.json,
agenda, .ics) and ingests proposal JSON files dropped into
`proposals/inbox/` when they carry a valid agent token. `satchel serve` exposes
the same command schema to agents at `POST /api/agent/rpc`.

## Consequences

- MCP over HTTP is not implemented yet (needed for remote agents in M7).
- The mirror is never read back as truth, except validated proposal files.
