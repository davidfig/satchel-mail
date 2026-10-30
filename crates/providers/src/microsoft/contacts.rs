//! Outlook / Microsoft 365 contacts through Microsoft Graph (the default
//! contacts folder as one address book).
//!
//! Needs the `Contacts.ReadWrite` scope; accounts signed in before Satchel
//! asked for it get [`ContactsScopeMissing`] until they reconnect. An update
//! reads the contact first: a change key that moved is a conflict, and only
//! the fields that changed are sent.

use std::collections::HashSet;

use anyhow::{Context, Result};
use async_trait::async_trait;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, ContactOp, ContactsProvider, ContactsScopeMissing, CredentialHandle, RemoteAddressBook, SyncCtx};
use serde_json::{Value, json};

use crate::oauth::ApiClient;

const ME: &str = "https://graph.microsoft.com/v1.0/me";
const SELECT: &str = "id,changeKey,displayName,emailAddresses,businessPhones,homePhones,mobilePhone,companyName,jobTitle,personalNotes,birthday";
const BOOK: &str = "contacts";

pub fn contact_to_fields(c: &Value) -> ContactFields {
    let s = |v: &Value| v.as_str().unwrap_or_default().trim().to_string();
    let mut phones = vec![];
    if let Some(m) = c["mobilePhone"].as_str().filter(|m| !m.trim().is_empty()) {
        phones.push(ContactPhone { number: m.trim().into(), label: Some("cell".into()) });
    }
    for (key, label) in [("businessPhones", "work"), ("homePhones", "home")] {
        for p in c[key].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|p| !p.trim().is_empty()) {
            phones.push(ContactPhone { number: p.trim().into(), label: Some(label.into()) });
        }
    }
    ContactFields {
        name: s(&c["displayName"]),
        emails: c["emailAddresses"].as_array().into_iter().flatten().filter_map(|e| e["address"].as_str()).filter(|e| !e.trim().is_empty()).map(|e| ContactEmail { email: e.trim().into(), label: None }).collect(),
        phones,
        organization: s(&c["companyName"]),
        title: s(&c["jobTitle"]),
        birthday: c["birthday"].as_str().and_then(|b| satchel_ical::vcard::normalize_birthday(b, None)),
        notes: s(&c["personalNotes"]),
    }
}

pub fn fields_to_contact(f: &ContactFields) -> Value {
    let words: Vec<&str> = f.name.split_whitespace().collect();
    let (given, surname) = match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => (rest.join(" "), last.to_string()),
        Some((only, _)) => (only.to_string(), String::new()),
        None => (String::new(), String::new()),
    };
    let numbers = |label: &str| f.phones.iter().filter(|p| p.label.as_deref() == Some(label)).map(|p| p.number.clone()).collect::<Vec<_>>();
    let mobile = f.phones.iter().find(|p| p.label.as_deref() == Some("cell")).map(|p| p.number.clone());
    // Graph has three kinds of number; anything else goes with home.
    let mut home = numbers("home");
    home.extend(f.phones.iter().filter(|p| !matches!(p.label.as_deref(), Some("cell" | "work" | "home"))).map(|p| p.number.clone()));
    // Graph holds at most three addresses and two business and two home numbers.
    let mut business = numbers("work");
    business.truncate(2);
    home.truncate(2);
    let mut body = json!({
        "displayName": f.name,
        "givenName": given,
        "surname": surname,
        "emailAddresses": f.emails.iter().take(3).map(|e| json!({ "address": e.email, "name": f.name })).collect::<Vec<_>>(),
        "mobilePhone": mobile,
        "businessPhones": business,
        "homePhones": home,
        "companyName": f.organization,
        "jobTitle": f.title,
        "personalNotes": f.notes,
    });
    // Graph needs a year; a year-less birthday is left out, not cleared.
    match f.birthday.as_deref() {
        Some(b) if b.starts_with("--") => {}
        Some(b) => body["birthday"] = json!(format!("{b}T00:00:00Z")),
        None => body["birthday"] = Value::Null,
    }
    body
}

/// An update to the server's contact `c` with only the properties that
/// changed. Unchanged names stay as Outlook has them: rebuilding given name
/// and surname from the display name gets "Mary Ann Smith" wrong.
pub fn update_body(c: &Value, new: &ContactFields) -> Value {
    let old = contact_to_fields(c);
    let full = fields_to_contact(new);
    let mut body = serde_json::Map::new();
    let mut take = |keys: &[&str]| {
        for k in keys {
            if let Some(v) = full.get(*k) {
                body.insert((*k).to_string(), v.clone());
            }
        }
    };
    if old.name != new.name {
        take(&["displayName", "givenName", "surname"]);
    }
    if old.emails != new.emails {
        take(&["emailAddresses"]);
    }
    if old.phones != new.phones {
        take(&["mobilePhone", "businessPhones", "homePhones"]);
    }
    if old.organization != new.organization {
        take(&["companyName"]);
    }
    if old.title != new.title {
        take(&["jobTitle"]);
    }
    if old.notes != new.notes {
        take(&["personalNotes"]);
    }
    if old.birthday != new.birthday {
        take(&["birthday"]);
    }
    Value::Object(body)
}

fn card(c: &Value) -> Option<(String, Option<String>, String)> {
    let id = c["id"].as_str()?.to_string();
    let vcard = satchel_ical::vcard::build(None, &id, &contact_to_fields(c));
    Some((id, c["changeKey"].as_str().map(String::from), vcard))
}

fn scope_error(e: anyhow::Error) -> anyhow::Error {
    if e.to_string().contains("HTTP 403") {
        return anyhow::Error::new(ContactsScopeMissing);
    }
    e
}

pub struct GraphContactsProvider {
    api: ApiClient,
}

impl GraphContactsProvider {
    pub fn new(creds: CredentialHandle) -> Result<Self> {
        Ok(Self { api: crate::microsoft::api_client(creds)? })
    }

    async fn get(&self, url: &str, what: &str) -> Result<Value> {
        super::graph_link(url)?;
        self.api.json(|h, t| h.get(url).header("Prefer", "IdType=\"ImmutableId\"").bearer_auth(t), what).await
    }
}

#[async_trait]
impl ContactsProvider for GraphContactsProvider {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let map = ctx.set_address_books(&[RemoteAddressBook { remote_id: BOOK.into(), name: "Outlook Contacts".into(), read_only: false }])?;
        let local = &map[BOOK];
        // Contact lists are small; list them and compare change keys.
        let known = ctx.store.card_remote_ids(local)?;
        let mut seen = HashSet::new();
        let mut url = format!("{ME}/contacts?$top=500&$select={SELECT}");
        loop {
            let v = self.get(&url, "list contacts").await.map_err(scope_error)?;
            for c in v["value"].as_array().cloned().unwrap_or_default() {
                let Some((id, etag, vcard)) = card(&c) else { continue };
                seen.insert(id.clone());
                if known.get(&id).is_none_or(|(_, e)| *e != etag) {
                    ctx.put_card(local, &id, etag.as_deref(), &vcard)?;
                }
            }
            match v["@odata.nextLink"].as_str() {
                Some(n) => url = super::graph_link(n)?,
                None => break,
            }
        }
        for gone in known.into_keys().filter(|k| !seen.contains(k)) {
            ctx.delete_card_remote(local, &gone)?;
        }
        Ok(())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ContactOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            ContactOp::Put { card_remote_id, vcard, etag, .. } => {
                let fields = satchel_ical::vcard::parse(vcard).map(|c| c.fields).context("invalid vCard")?;
                let v = match card_remote_id {
                    None => {
                        let body = fields_to_contact(&fields);
                        self.api.json(|h, t| h.post(format!("{ME}/contacts")).header("Prefer", "IdType=\"ImmutableId\"").bearer_auth(t).json(&body), "create contact").await.map_err(scope_error)?
                    }
                    Some(id) => {
                        let current = self.get(&format!("{ME}/contacts/{}?$select={SELECT}", crate::net::enc(id)), "get contact").await?;
                        if etag.is_some() && current["changeKey"].as_str() != etag.as_deref() {
                            return Err(ApiError::new(ErrorCode::Conflict, "the contact changed in Outlook").into());
                        }
                        let body = update_body(&current, &fields);
                        if body.as_object().is_some_and(|o| o.is_empty()) {
                            current
                        } else {
                            self.api.json(|h, t| h.patch(format!("{ME}/contacts/{}", crate::net::enc(id))).header("Prefer", "IdType=\"ImmutableId\"").bearer_auth(t).json(&body), "update contact").await?
                        }
                    }
                };
                if let Some((id, etag, vcard)) = card(&v) {
                    out.card_remote_id = Some(id);
                    out.card_etag = etag;
                    out.card_vcard = Some(vcard);
                }
            }
            ContactOp::Delete { card_remote_id, .. } => match self.api.send(|h, t| h.delete(format!("{ME}/contacts/{}", crate::net::enc(card_remote_id))).bearer_auth(t), "delete contact").await {
                Ok(_) => {}
                Err(e) if e.to_string().contains("HTTP 404") => {}
                Err(e) => return Err(e),
            },
        }
        Ok(out)
    }

    async fn fetch(&self, _ctx: &SyncCtx, _book: &str, card_remote_id: &str) -> Result<Option<(Option<String>, String)>> {
        match self.get(&format!("{ME}/contacts/{}?$select={SELECT}", crate::net::enc(card_remote_id)), "get contact").await {
            Ok(v) => Ok(card(&v).map(|(_, etag, vcard)| (etag, vcard))),
            Err(e) if e.to_string().contains("HTTP 404") => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_round_trip() {
        let c = json!({
            "id": "AAMk1", "changeKey": "ck1", "displayName": "Ann Lee",
            "emailAddresses": [{ "address": "ann@x.test", "name": "Ann" }],
            "mobilePhone": "+1 555 0100", "businessPhones": ["+1 555 0200"], "homePhones": [],
            "companyName": "Acme", "jobTitle": "CTO", "personalNotes": "", "birthday": "1990-01-15T00:00:00Z"
        });
        let f = contact_to_fields(&c);
        assert_eq!(f.phones.len(), 2);
        assert_eq!(f.birthday.as_deref(), Some("1990-01-15"));
        let back = fields_to_contact(&f);
        assert_eq!(back["givenName"], "Ann");
        assert_eq!(back["surname"], "Lee");
        assert_eq!(back["mobilePhone"], "+1 555 0100");
        assert_eq!(back["businessPhones"][0], "+1 555 0200");
        assert_eq!(back["birthday"], "1990-01-15T00:00:00Z");
        assert_eq!(contact_to_fields(&back), f);

        // Only what changed is sent; names and a year-less birthday are left alone.
        let mut c2 = c.clone();
        c2["displayName"] = json!("Mary Ann Smith");
        c2["givenName"] = json!("Mary Ann");
        let mut edited = contact_to_fields(&c2);
        edited.title = "CEO".into();
        edited.birthday = Some("--01-15".into());
        let body = update_body(&c2, &edited);
        assert_eq!(body, json!({ "jobTitle": "CEO" }));
        assert_eq!(update_body(&c2, &contact_to_fields(&c2)), json!({}));
    }
}
