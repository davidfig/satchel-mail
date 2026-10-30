//! iCalendar (RFC 5545) parsing and serialization for the subset Satchel
//! needs, plus recurrence expansion and iMIP reply generation. vCard, which
//! shares the content-line syntax, lives in [`vcard`].

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use satchel_api::{Address, Attendee, Event, EventStatus, PartStat};

mod event;
mod itip;
mod lines;
mod recur;
mod serialize;
#[cfg(test)]
mod tests;
mod time;
pub mod vcard;

pub use event::*;
pub use itip::*;
use lines::*;
pub use lines::{fold, unfold};
pub use recur::*;
pub use serialize::*;
pub use time::*;
