//! Contacts: people generated from the mail you exchange, edits and merges,
//! and reconciling server address books. See docs/decisions/0026-contacts.md.
//!
//! Rules that keep the user in charge:
//! - automatic generation only ever touches contacts whose origin is `auto`;
//!   any edit by the user makes a contact `local`;
//! - deleted contacts stay as tombstones with their addresses, so automatic
//!   generation never brings them back;
//! - server changes are merged field by field against the last version the
//!   server confirmed, and where the user changed a field, their value wins.

mod auto;
mod cards;
mod commands;
mod merge;
#[cfg(test)]
mod tests;

pub use auto::*;
pub(crate) use cards::*;
pub use merge::*;

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::Utc;
use satchel_api::*;
use serde_json::json;

use crate::engine::{Caller, Engine};
use crate::error::{forbidden, invalid, not_found};
use crate::store::{Correspondent, Evidence, Store, StoredContact, ms_to_dt};
