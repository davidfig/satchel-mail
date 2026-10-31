//! Calendars, events, attendees and free time.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Calendar {
    pub id: Id,
    pub account_id: Id,
    pub name: String,
    pub color: String,
    pub read_only: bool,
    /// Whether events in this calendar are shown.
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Confirmed,
    Tentative,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartStat {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attendee {
    pub email: String,
    pub name: Option<String>,
    pub status: PartStat,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Local id; None when creating.
    pub id: Option<Id>,
    pub calendar_id: Id,
    /// iCalendar UID.
    #[serde(default)]
    pub uid: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub all_day: bool,
    /// IANA zone the event was authored in.
    #[serde(default)]
    pub timezone: Option<String>,
    /// RFC 5545 RRULE value, e.g. "FREQ=WEEKLY;BYDAY=MO,WE".
    #[serde(default)]
    pub rrule: Option<String>,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    #[serde(default)]
    pub organizer: Option<Address>,
    #[serde(default = "default_status")]
    pub status: EventStatus,
    /// The account owner's own participation status, if invited.
    #[serde(default)]
    pub my_status: Option<PartStat>,
    /// Reminders that show on screen. Always set when read; on writes,
    /// `None` keeps the stored ones (0056).
    #[serde(default)]
    pub alerts: Option<Vec<Alert>>,
}

/// A reminder of an event, relative to its start (0056).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    /// Minutes before the start; negative is after it. All-day events
    /// count from midnight at the start of their first day.
    pub minutes_before: i32,
}

/// An alert that has gone off and hasn't been dismissed, or (from
/// `ListUpcomingAlerts`) one that will.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarAlert {
    /// `{event_id}:{occurrence start, ms}:{minutes_before}`: known before it goes off.
    pub id: Id,
    pub event_id: Id,
    pub calendar_id: Id,
    pub title: String,
    pub location: String,
    /// The occurrence the alert is for.
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub color: String,
    pub minutes_before: i32,
    /// When it goes off: the alert's time, or (if snoozed) the snooze's end.
    pub due: DateTime<Utc>,
    pub snoozed: bool,
}

fn default_status() -> EventStatus {
    EventStatus::Confirmed
}

/// One concrete occurrence of an event (recurrences expanded).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Occurrence {
    pub event: Event,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub color: String,
}

/// Where a place suggestion came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceSource {
    /// A location already used in one of your events.
    History,
    OpenStreetMap,
    AppleMaps,
}

/// A suggestion for an event's location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceSuggestion {
    /// What goes in the location field.
    pub text: String,
    /// The place's name, shown first.
    pub label: String,
    /// The address or area, shown muted.
    pub detail: Option<String>,
    pub source: PlaceSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeSlot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}
