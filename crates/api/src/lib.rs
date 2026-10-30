//! The command/event schema every satchel.email host speaks.
//!
//! The UI (over Tauri IPC or HTTP+WebSocket) and the MCP server both talk to the
//! core exclusively through [`Command`] and [`CoreEvent`]. If it isn't expressible
//! here, no client can do it — which keeps the desktop app, the CLI and the
//! future hosted service at feature parity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type Id = String;

mod account;
mod agents;
mod calendar;
mod command;
mod contacts;
mod event;
mod mail;
mod wire;

pub use account::*;
pub use agents::*;
pub use calendar::*;
pub use command::*;
pub use contacts::*;
pub use event::*;
pub use mail::*;
pub use wire::*;
