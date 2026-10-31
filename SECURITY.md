# Security policy

## Reporting a problem

Please report security problems privately, not in a public issue: use **Report a vulnerability** on the repository's Security tab. If you can't, email [hello@satchel.email](mailto:hello@satchel.email). Include what an attacker could do, the steps or a message that shows it, and the version or commit you tested.

satchel.email is maintained by one person, so replies are best effort, usually within a week. You'll hear when the report is confirmed and when a fix ships; tell me if you'd like credit in the release notes.

## What's covered

satchel.email reads untrusted mail and lets AI agents in, so these matter most:

- a message, invite or attachment that runs script, reaches the network, crashes or hangs the app, or corrupts the local store;
- an agent (through MCP or the `~/Satchel/` folder) doing more than read and propose: sending, deleting, changing settings or its own policy;
- reading or writing files outside the data directory and the AI folder;
- leaking stored credentials or OAuth tokens;
- `satchel serve` or the desktop app accepting requests from other sites or other users on the machine.

[docs/decisions/0032-local-attack-surface.md](docs/decisions/0032-local-attack-surface.md) describes the local threat model. Fixes go into the latest release only.

## Please don't

Test against mail accounts that aren't yours. `scripts/dev-servers reset` starts a local mail server with test accounts for this.
