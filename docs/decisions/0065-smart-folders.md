# 0065. Smart folders: saved searches under Mail

- **Status:** Accepted
- **Date:** 2026-10-29

## Context
People want standing views that cut across folders and accounts: unread mail
from a few people, everything with attachments this week, receipts. Other
clients call these Smart Mailboxes (Apple Mail), saved searches or virtual
folders (Thunderbird, Fastmail), Search Folders (Outlook) and splits
(Superhuman). All of them store a query, not messages, and run it again when
the folder opens. The ones that use the search language (Fastmail,
Superhuman) needed negation and relative dates for saved searches to stay
useful. Search here (0014) already turns operators and free text into one SQL
query with counts and paging. It had no negation or relative dates, and
`from:a from:b` meant either one.

## Decision
- **A smart folder is a name, a query in the search box's language, and what
  its badge counts** (unread conversations, all, or nothing). It lives in a
  local `smart_folders` table (schema 9) and isn't synced to the servers,
  since no protocol has a place for it. It stores no mail, and deleting it
  touches none.
- **It searches every folder of every account.** Trash and Junk are left out
  unless the query says `in:anywhere` (the form's "Include Trash and Spam"),
  as Apple Mail does. `account:` and `in:` narrow it. It lists conversations,
  so a thread in several folders appears once.
- **The search language grew:**
  - A leading `-` leaves out any thread with a message matching the term,
    including free text (`-from:noreply`, `-draft`). A quoted `in:` value is
    one folder's name, so it can contain a comma.
  - `is:read` and `is:unflagged` are the negations of `is:unread` and
    `is:flagged`. `is:favorite` (0053) works in a folder like any other term.
  - Relative dates are worked out when the query runs: `newer_than:7d` and
    `older_than:1y` (d, w, m = 30 days, y = 365 days).
  - `in:` takes a list, `in:anywhere` includes Trash and Junk, and `account:`
    matches part of an account's address or name.
  - Every term must hold. A comma list inside one operator is still "any of"
    (`from:a,b`), but `from:a from:b` now means both, as in Gmail. Before, it
    meant either.
  - Plain terms apply to the messages of a thread: a thread matches when one
    of its messages meets all of them. Negated terms apply to the whole thread:
    none of its messages may match, or `-in:inbox` would keep every Inbox thread
    with a reply in Sent, and `is:read` every part-read thread. Only messages
    the filter may show count, so a message outside an agent's date limit can't
    be detected this way.
- **One command surface:**
  - `ListSmartFolders` returns the folders with their counts, worked out on
    request. The UI asks again, debounced, when mail changes.
  - `SaveSmartFolder`, `DeleteSmartFolder` and `ReorderSmartFolders` change
    them and emit `SmartFoldersChanged`.
  - `ThreadQuery.smart_folder_id` runs a folder's query in the core, and a
    `search` alongside it narrows the folder. So the list, the counts, the
    CLI, MCP and the AI folder all read the same definition.
- **Agents can read smart folders but never change them** (0006): changing
  them is a user setting. A folder's name and query can describe any account's
  mail, so an agent limited to some accounts gets none, and an AI folder limited
  to some accounts has no `smart-folders.md`. Other agents get `smart-folders.md` in the AI folder (each
  folder's query and newest threads, linked to the thread files), MCP's
  `list_smart_folders`, and `smart_folder_id` on `search_threads`.
- **In the UI:**
  - Smart folders are in Mail's sidebar section, above the accounts, with "New
    smart folder" after them. The mode rows above never move (0051).
  - A search can be saved from the list header.
  - The edit form's fields (From, To, Subject, words, flags, favorites, received,
    accounts, include Trash) write the query. Typing a query fills in the
    fields it can, and terms the form has no field for are kept as typed.
  - Right-click to edit, move up or down, or delete. Double-click to edit.
  - The command palette lists each smart folder.

## Consequences
- An unread badge counts conversations with any unread mail, including when
  the message that matched is read.
- Each badge is one count query per refresh, on the background read connection
  (0038) so it doesn't hold up the user's reads. If many smart folders make the
  sidebar slow, cache counts in a worker instead. A count that fails shows no
  badge rather than failing the list.
- `smart-folders.md` is rewritten when mail or the folders change, or every 10
  minutes to keep relative dates current, not on every AI folder pass.
- Smart folders don't follow the user to another computer until the hosted
  version (0018) syncs settings.
- Not done yet: OR across different operators and grouping (Superhuman's `{}` and
  `()`), dragging to reorder, and agents proposing a new smart folder.
