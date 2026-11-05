# 0041. Changing the AI folder's location moves it

- **Status:** Accepted
- **Date:** 2026-10-30

## Context
The AI folder defaults to `~/Satchel/`, a plain folder in the home folder that
any AI tool can be given without a macOS privacy prompt (unlike `~/Documents`,
Desktop or Downloads). Some users still want it elsewhere, such as in a synced
or already-shared folder. Until now the location was a text box: `~` wasn't
expanded, and saving a new location deleted the old `mail/` and wrote
everything again at the new place. That left behind proposals waiting in
`proposals/inbox/`, the processed log and any files agents kept in the folder.

## Decision
- The desktop app has a **Choose…** button. A picked folder gets a `Satchel`
  folder inside it (unless it's already named `Satchel`), so picking
  Documents never mixes mail into the user's own files. The text box stays,
  and is all the web UI has.
- `~` at the start of the path means the home folder.
- Saving a new location **moves** the folder (`ai_folder::relocate`): a rename,
  or a copy then delete across disks. Symlinks are skipped, not followed.
- The new location must be absent or an empty folder, a full path, and not
  inside the old one (or the other way round). Otherwise the save fails and
  nothing changes.
- The move and each mirror pass hold `Engine::mirror_lock`, so a pass can't
  write into the old location while it moves. The move is logged in the audit
  log.

## Consequences
- Pending proposals and agents' own files survive a move.
- AI tools still pointed at the old path see nothing there; the UI says to
  point them at the new location.
- Proposal ingestion runs outside the lock; a file being processed at the
  instant of a move could land its processed copy in the old location.
