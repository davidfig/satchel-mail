//! MIME helpers: parse incoming mail into Satchel's model, sanitize HTML,
//! extract attachments, build outgoing messages, and quote/strip replies.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use mail_parser::{MessageParser, MimeHeaders};
use satchel_api::{Address, Attachment, Unsubscribe};
mod build;
mod html;
mod parse;
mod rebuild;
mod safety;
mod slim;
#[cfg(test)]
mod tests;
mod text;

pub use build::*;
pub use html::*;
pub use parse::*;
pub use rebuild::{Body, Part, Slim, header_block, keeps_contents, rebuild};
pub use safety::{auth_failure, base_domain, deceptive_links, email_domain, is_risky_attachment, parse_mailto, parse_unsubscribe};
pub use slim::{KEEP_INLINE_BYTES, SLIM_FROM_BYTES, strip_attachments};
pub use text::*;
