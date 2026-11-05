# 0045. Agents can propose merging contacts

- **Status:** Accepted
- **Date:** 2026-10-30
- **Amends:** 0029

## Context

Real address books are full of duplicates: the same person under a work and
a home address, or two cards for one person in the same Outlook address book.
Satchel doesn't merge these on its own (0026): different addresses can belong
to different people, and a name match proves nothing. The user can merge
contacts by hand, but finding the duplicates among hundreds of people is
tedious, and it is exactly the kind of work an agent reading the mail is good
at — it can see that two addresses sign with the same name and write about
the same things.

0029 said agents can't delete or merge contacts. That kept a prompt-injected
agent from quietly folding a stranger's address into someone the user trusts.

## Decision

Agents may **propose** a merge. A new proposal action, `merge_contacts`,
carries the contact ids to merge; the first is kept and the others are merged
into it. In the AI folder it is a proposal file:

    ---
    action: merge_contacts
    contacts: [<contact_id to keep>, <contact_id>]
    why: Same person, writing from both addresses
    ---

Over MCP it is the `propose_merge_contacts` tool, taking `contact_ids` in the
same order.

- It goes through `CreateProposal` like everything else.
  `Command::allowed_for_agent` doesn't change: agents still can't call
  `MergeContacts` themselves.
- It needs the agent policy's `allow_contacts`, and every contact must be one
  the agent may see (an address book in one of its accounts).
- The proposal always carries warnings: that merging can't be undone from
  Satchel, the addresses added to the kept contact, and different names when
  they differ. So under 0023 it waits in Review unless the user has let risky
  changes through.
- The contacts' versions are recorded, so if any of them changes before the
  user approves, the proposal goes stale and has to be reviewed again.
- Approving runs the same merge as `MergeContacts`: server cards are linked
  to the kept contact, not deleted.
- Review shows each contact, the one to keep first.

Deleting contacts stays off-limits to agents.

## Consequences

- Agents can clean up duplicate people, with the user deciding each merge.
- A merge is the first agent-proposable contact change that removes contacts
  from the list, so the warnings matter: the review shows every address that
  will join the kept contact.
- A merge can't be undone from Satchel. The cards are kept on the server, so
  the data isn't lost, but splitting a merged person back up is manual.
