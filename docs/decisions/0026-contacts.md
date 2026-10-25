# 0026. Contacts: generated from mail, edited by you, synced with your accounts

- **Status:** Accepted (merging and pushing edits amended by 0033)
- **Date:** 2026-10-24

## Context

Satchel had no contacts. The composer suggested addresses from senders, and
the spam code kept its own list of people you've written to. People want an
address book that fills itself in, that they can correct, and that matches
the address books on their accounts (CardDAV, Google, Outlook). A later
"person view" will show all mail with someone across every address they use,
so one person with many addresses has to be a first-class idea, not a
display trick.

Two things pull against each other. Automation should do the boring work.
But the user's corrections must stick: an automatic pass must never undo a
rename, re-add an address the user removed, or bring back a deleted contact.

## Decision

**Model.** A *contact* is one person: a name, any number of email addresses,
and phones, organization, title, birthday and notes. These fields map onto
vCard properties (FN/N, EMAIL, TEL, ORG, TITLE, BDAY, NOTE), so a contact can
round-trip through any address book. A contact can have a *card* in zero or
more server address books, one card per address book. A card keeps the vCard
the server last confirmed. When we write a card back, we patch that vCard:
photos, postal addresses, URLs and X- properties are kept as they were, and
unchanged properties keep their original line. The vCard code is in-house
(`satchel-ical::vcard`, which shares the content-line syntax with iCalendar),
so no new dependency. Contacts, cards and address books live in
`store.rs`/`schema.sql`.

Each contact has an *origin*: `auto` (generated, untouched by the user),
`local` (created or edited in Satchel) or `remote` (came from a server).

**Automatic contacts.** A pass works out your correspondents from the whole
mail store and creates one `auto` contact for each correspondent no contact
already has. It runs at startup (that's the backfill over existing mail),
after syncs, and at most every 30 seconds. A correspondent is someone you:

- sent mail to (To, Cc or Bcc; drafts don't count), or
- answered, or wrote to in the same thread (this covers Gmail, which doesn't
  mark messages answered).

Excluded:

- your own addresses;
- senders you marked as spam (an existing auto contact for them is removed);
- mail in Spam folders;
- machine addresses: no-reply, mailer-daemon, bounces, notifications,
  newsletters, per-thread `reply+…` addresses, long tokenized local parts.

One-off senders you never wrote back to never qualify. The name is the one
the person uses for themselves in their own From line, if they gave one;
otherwise the name others use. We don't guess merges by name, because two
different people can share a name. Grouping addresses is the user's merge.

**User edits win.**

- The automatic pass only changes contacts whose origin is `auto` (it keeps
  their name current). Any edit makes a contact `local`.
- Deleting a contact keeps a *tombstone*: the row is hidden, but its addresses
  stay claimed, so the pass never brings it back. It is also deleted from
  every server address book it was in.
- Merging moves the other contacts' addresses, numbers, cards and notes into
  the first contact and removes the others. Their addresses stay claimed by
  the merged contact.

**Sync.** It mirrors calendars:

- A `ContactsProvider` trait (sync, apply, fetch) writes through `SyncCtx`.
  Changes go out through the ops queue as `contact_put` and
  `contact_delete`, in vCard, the way calendar ops speak iCalendar.
- Implementations:
  - **CardDAV** (RFC 6352; sync-collection with an ETag fallback; If-Match
    and If-None-Match on writes). Its URL comes from a new optional
    `carddav_url`; if that's empty, it is discovered from the CalDAV server's
    principal, then from `/.well-known/carddav`.
  - **Google Contacts** through the People API, with sync tokens.
  - **Outlook contacts** through Graph: the default folder, listed and
    compared by changeKey.
- Address books sync every 5 minutes and on "Sync now". A failure is only
  logged; it never turns the account red.

How server cards relate to contacts:

- **A new card links to an existing contact** if they share an address and
  that contact has no card in the same address book. If the contact was
  `auto`, the card replaces the guess. Otherwise the card only fills gaps and
  adds addresses. This is how "Bob" from Gmail contacts and "Bob" from your
  mail become one person.
- **A changed card is merged field by field.** The base is the stored vCard,
  ours is the contact, theirs is the new card. A field the user changed keeps
  the user's value; other fields take the server's. Address and phone lists
  merge as sets: their additions and removals apply, ours are kept.
- **A conflicting write** (HTTP 412, or Google's FAILED_PRECONDITION) fetches
  the server copy, merges it the same way, and retries once.
- **A card deleted on the server** removes the person, as a tombstone, unless
  another address book still has them. If the user edited a card that was
  deleted on the server, the edit wins and the card is recreated.
- **A user edit is pushed to every card** of the contact. Satchel never
  writes to a server card except as the result of a user action: edit,
  merge, delete, or approving a proposal.

**Automatic contacts stay local.** They are never pushed on their own. They
can number in the hundreds, would clutter address books on every device, and
would send a list of everyone you email to each provider. Providers already
keep their own "other contacts" for that. When the user saves a contact,
the form offers to save it to an address book. New contacts default to "Only
in Satchel".

**OAuth scopes.**

- New Google sign-ins request `https://www.googleapis.com/auth/contacts`.
  New Microsoft sign-ins request `Contacts.ReadWrite`.
- Existing accounts are **not** forced to sign in again. When the provider
  answers 403 for contacts, sync skips contacts for that account and records
  a flag (`Account.contacts_need_reconnect`). Settings → Accounts then shows
  "Reconnect to sync contacts". Signing in again with the same account
  updates its credential instead of failing as a duplicate.
- Graph has no conditional update we use for contacts, so the last write
  wins there.

**Agents.**

- Agents read contacts through `ListContacts`, `GetContact` and
  `ListAddressBooks`. The AI folder writes one file per person,
  `contacts/people/<name>-<id>.md`, and `contacts/CONTACTS.md` listing them;
  MCP adds `search_contacts` and `get_contact`. (Updated 2026-10-24: this
  replaced a single `contacts.json`. A file per person stays small to read
  and diff, a new contact adds a file instead of rewriting a big one, and
  each file names the accounts and address books the person is saved in,
  so an agent can tell that Bob is in both the Gmail and Outlook address
  books. Agents can edit or add these files to propose changes: 0029.)
- They may only *propose*, with `ProposalAction::SaveContact` (create or
  change; never delete or merge). It fits the existing machinery: version
  staleness, Review, `save_contact` proposal files, and MCP `propose_contact`.
- These proposals carry risk warnings: adding an address to an existing
  contact, renaming one, or creating a contact named like an existing one.
  An injected agent that quietly adds `attacker@…` to "Bob" could misdirect
  mail through composer suggestions, so under 0023 such proposals always
  wait for review.
- A new `AgentPolicy.allow_contacts` setting controls access. Policies saved
  before it existed take the agent's `allow_calendar` value.
- Agents limited to some accounts, and an AI folder limited the same way,
  only see contacts saved in those accounts' address books. Automatic and
  local contacts come from every account's mail, so they would leak across
  accounts.

## Consequences

- The composer suggests contacts first, by name or any address.
- The person view (0027) covers all of a contact's addresses, and a contact's
  message count is the person view's received + sent (`Store::address_stats`),
  so the two always agree.
- Spam impersonation checks still use their own list of people you write to.
  Feeding contact names into them is a natural next step, but agent-proposed
  contacts would then need care.
- Correspondents are worked out from all mail on each pass. That is cheap at personal
  mail volumes; if it ever isn't, make it incremental on message rowid.
- Postal addresses, photos and other vCard fields aren't editable, but they
  survive round trips. Adding them means extending `ContactFields` and the
  vCard patcher.
