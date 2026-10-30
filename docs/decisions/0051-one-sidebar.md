# 0051. One sidebar: fixed mode rows, each view's own section below

- **Status:** Accepted
- **Date:** 2026-10-30

## Context
Switching between Mail, Calendar and Contacts was harder than it should be.
With more than one account, the combined Inbox/Flagged/… list sat between
Mail and Calendar, so the mode rows moved depending on the mailboxes. Calendar
also had a second side column of its own (New event, a month, the calendar
list) next to the app sidebar, so it showed two New event buttons, and the day
view repeated the month in its side panel.

## Decision
The sidebar's nav rows (Mail, Calendar, Contacts, AI Proposals, Settings) come
first and never move. Below them goes the current view's own section: Mail's
combined list and account folders, or a section the view supplies as `side`
(an `<aside>` it owns and patches itself; the shell only places it). Calendar
supplies its month and calendar list there, in every mode. The month marks
busy days in day view, whose side panel now shows only the day's agenda. The
calendar list folds away under its header (shut by default, remembered in
local storage). Collapsing the sidebar (⌘\) hides the whole section;
Calendar's own `[` toggle is gone.

## Consequences
The grid gets the width the second column took, and there's one New event
button. Contacts (or any view) can add a sidebar section the same way. The
month can't be hidden without collapsing the whole sidebar.
