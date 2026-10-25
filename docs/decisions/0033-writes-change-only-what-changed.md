# 0033. Writes to the server change only what the user changed

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

Satchel models a simplified view of calendar events and contacts. Each
write sent that whole view back, so anything Satchel doesn't model was
replaced or lost:

- **Outlook events:** editing an event's time also replaced its body with
  the ~255-character preview (dropping the Teams link) and its location with
  a bare display name.
- **Google Contacts:** an edit replaced names and organizations wholesale,
  dropping structured names, departments and extra organizations.
- **Outlook contacts:** an edit rebuilt given name and surname from the
  display name, and a birthday without a year cleared the server's.
- **Merging contacts** deleted the extra server cards, losing photos and
  postal addresses.
- **Edits to a person** with cards in several address books copied the
  pooled fields into every card, so personal numbers reached a work address
  book.
- **Google and Outlook event edits** never checked whether the event had
  changed on the server first.
- **Queued IMAP ops** could act on UIDs from before the server renumbered a
  folder.

## Decision

- **Calendar edits read the event first.** Outlook: the changeKey must match
  the one from the last sync, or the edit fails as a conflict. Body,
  location and attendees are sent only if they differ from the server's.
  Google: the PATCH carries `If-Match`, and only our own attendee status is
  sent. The organizer comes from Outlook's `isOrganizer`, not by comparing
  addresses.
- **Outlook RSVPs:** a response is sent only when our answer differs from
  Outlook's. Answering an occurrence answers its series.
- **Contact edits apply a change, not a copy.** An edit is queued with the
  contact as it was before. Each card gets that change three-way merged onto
  its own content (`merge_fields(before, now, card)`), so cards keep their
  own numbers and notes. Google and Outlook writers read the current
  contact, fail on a changed etag or changeKey, and send only the fields
  that differ, editing the primary organization in place.
- **Merging links cards.** It never deletes or rewrites them; the person may
  keep duplicate cards on the server. This amends the merge rules in 0026.
- **IMAP ops check UIDVALIDITY.** A uid recorded under another UIDVALIDITY is
  skipped, not acted on. A folder rename rekeys its messages' remote ids.
- **JMAP drafts:** a server draft is destroyed only while it still has
  `$draft`. A draft sent from another client keeps its id.

## Consequences

- Writes cost one extra read. Edits made in Satchel and elsewhere at the same
  time conflict instead of one silently overwriting the other.
- Duplicate cards stay on the server after a merge. Use the provider's own
  duplicate merging to remove them.
