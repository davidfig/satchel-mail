//! Contacts and address books.

use super::*;

/// An address book on the server (CardDAV collection, Google Contacts, Outlook contacts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressBook {
    pub id: Id,
    pub account_id: Id,
    pub name: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactEmail {
    pub email: String,
    /// vCard TYPE, lowercase: "home", "work", "other"...
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactPhone {
    pub number: String,
    /// vCard TYPE, lowercase: "cell", "home", "work"...
    #[serde(default)]
    pub label: Option<String>,
}

/// The editable part of a contact. Maps onto vCard (RFC 6350) properties:
/// FN, EMAIL, TEL, ORG, TITLE, BDAY, NOTE. Other vCard properties on synced
/// cards are kept as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactFields {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub emails: Vec<ContactEmail>,
    #[serde(default)]
    pub phones: Vec<ContactPhone>,
    #[serde(default)]
    pub organization: String,
    #[serde(default)]
    pub title: String,
    /// `YYYY-MM-DD`, or `--MM-DD` without a year.
    #[serde(default)]
    pub birthday: Option<String>,
    #[serde(default)]
    pub notes: String,
}

/// Where a contact came from. Only `auto` contacts are ever changed by
/// automatic generation; any edit by the user makes them `local`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactOrigin {
    /// Generated from mail you exchanged, not edited since.
    Auto,
    /// Created or edited in Satchel.
    #[default]
    Local,
    /// Came from an account's address book.
    Remote,
}

/// A server address book a contact is saved in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactSource {
    pub address_book_id: Id,
    pub account_id: Id,
    pub name: String,
}

/// One person: a name and any number of addresses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contact {
    /// Local id; None when creating.
    #[serde(default)]
    pub id: Option<Id>,
    #[serde(flatten)]
    pub fields: ContactFields,
    // Read-only, filled in by the core.
    #[serde(default)]
    pub origin: ContactOrigin,
    #[serde(default)]
    pub sources: Vec<ContactSource>,
    /// Messages from or sent to any of this contact's addresses (drafts
    /// excluded); the same count as the person view.
    #[serde(default)]
    pub message_count: u32,
    #[serde(default)]
    pub last_contacted: Option<DateTime<Utc>>,
    /// Bumped on every change (proposal staleness).
    #[serde(default)]
    pub version: i64,
}
