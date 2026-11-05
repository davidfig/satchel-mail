//! Google Contacts through the People API ("My Contacts" as one address book).
//!
//! Needs the `contacts` OAuth scope; accounts signed in before Satchel asked
//! for it get [`ContactsScopeMissing`] and are skipped until they reconnect.

use std::collections::HashSet;

use anyhow::{Context, Result};
use async_trait::async_trait;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, ContactOp, ContactsProvider, ContactsScopeMissing, CredentialHandle, RemoteAddressBook, SyncCtx};
use serde_json::{Value, json};

use crate::oauth::ApiClient;

const PEOPLE: &str = "https://people.googleapis.com/v1";
const FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations,birthdays,biographies,metadata";
const UPDATE_FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations,birthdays,biographies";
/// Google has one address book per account.
const BOOK: &str = "people/me";

fn primary(v: &Value) -> Option<&Value> {
    let arr = v.as_array()?;
    arr.iter().find(|x| x["metadata"]["primary"] == true).or(arr.first())
}

fn label(t: &Value) -> Option<String> {
    let t = t.as_str()?.trim().to_lowercase();
    match t.as_str() {
        "" => None,
        "mobile" => Some("cell".into()),
        _ => Some(t),
    }
}

/// A People API person as contact fields.
pub fn person_to_fields(p: &Value) -> ContactFields {
    let s = |v: &Value| v.as_str().unwrap_or_default().trim().to_string();
    let org = primary(&p["organizations"]);
    let birthday = primary(&p["birthdays"]).and_then(|b| {
        let d = &b["date"];
        let (m, day) = (d["month"].as_u64()?, d["day"].as_u64()?);
        Some(match d["year"].as_u64() {
            Some(y) if y > 0 => format!("{y:04}-{m:02}-{day:02}"),
            _ => format!("--{m:02}-{day:02}"),
        })
    });
    ContactFields {
        name: primary(&p["names"]).map(|n| s(&n["displayName"])).unwrap_or_default(),
        emails: p["emailAddresses"].as_array().map(|a| a.iter().filter(|e| e["value"].is_string()).map(|e| ContactEmail { email: s(&e["value"]), label: label(&e["type"]) }).collect()).unwrap_or_default(),
        phones: p["phoneNumbers"].as_array().map(|a| a.iter().filter(|e| e["value"].is_string()).map(|e| ContactPhone { number: s(&e["value"]), label: label(&e["type"]) }).collect()).unwrap_or_default(),
        organization: org.map(|o| s(&o["name"])).unwrap_or_default(),
        title: org.map(|o| s(&o["title"])).unwrap_or_default(),
        birthday,
        notes: primary(&p["biographies"]).map(|b| s(&b["value"])).unwrap_or_default(),
    }
}

/// Contact fields as a People API person body.
pub fn fields_to_person(f: &ContactFields) -> Value {
    let typed = |label: &Option<String>, mobile: bool| -> Value {
        match label.as_deref() {
            Some("cell") if mobile => json!("mobile"),
            Some(l) => json!(l),
            None => Value::Null,
        }
    };
    let birthday = f.birthday.as_deref().and_then(|b| {
        let parts: Vec<&str> = b.trim_start_matches("--").split('-').collect();
        match parts.as_slice() {
            [y, m, d] => Some(json!({ "date": { "year": y.parse::<u32>().ok()?, "month": m.parse::<u32>().ok()?, "day": d.parse::<u32>().ok()? } })),
            [m, d] => Some(json!({ "date": { "month": m.parse::<u32>().ok()?, "day": d.parse::<u32>().ok()? } })),
            _ => None,
        }
    });
    json!({
        "names": if f.name.is_empty() { json!([]) } else { json!([{ "unstructuredName": f.name }]) },
        "emailAddresses": f.emails.iter().map(|e| json!({ "value": e.email, "type": typed(&e.label, false) })).collect::<Vec<_>>(),
        "phoneNumbers": f.phones.iter().map(|p| json!({ "value": p.number, "type": typed(&p.label, true) })).collect::<Vec<_>>(),
        "organizations": if f.organization.is_empty() && f.title.is_empty() { json!([]) } else { json!([{ "name": f.organization, "title": f.title }]) },
        "birthdays": birthday.map(|b| vec![b]).unwrap_or_default(),
        "biographies": if f.notes.is_empty() { json!([]) } else { json!([{ "value": f.notes, "contentType": "TEXT_PLAIN" }]) },
    })
}

/// An update to the server's `person` that changes only the fields that
/// differ from `new`, and the `updatePersonFields` that name them. Google
/// replaces every field it is sent, so an unchanged field sent back in our
/// simpler shape would lose what we don't model: structured and phonetic
/// names, departments, extra organizations.
pub fn update_body(person: &Value, new: &ContactFields) -> (Value, Vec<&'static str>) {
    let old = person_to_fields(person);
    let ours = fields_to_person(new);
    let mut body = json!({ "etag": person["etag"] });
    let mut fields = vec![];
    if old.name != new.name {
        body["names"] = ours["names"].clone();
        fields.push("names");
    }
    // Entries the server already has keep their own metadata.
    let reuse = |key: &str, ours: &Value| -> Value {
        let theirs = person[key].as_array().cloned().unwrap_or_default();
        let list = ours.as_array().cloned().unwrap_or_default();
        Value::Array(
            list.into_iter()
                .map(|o| match theirs.iter().find(|t| t["value"] == o["value"]) {
                    Some(t) => {
                        let mut t = t.clone();
                        t["type"] = o["type"].clone();
                        t
                    }
                    None => o,
                })
                .collect(),
        )
    };
    if old.emails != new.emails {
        body["emailAddresses"] = reuse("emailAddresses", &ours["emailAddresses"]);
        fields.push("emailAddresses");
    }
    if old.phones != new.phones {
        body["phoneNumbers"] = reuse("phoneNumbers", &ours["phoneNumbers"]);
        fields.push("phoneNumbers");
    }
    if (old.organization != new.organization) || (old.title != new.title) {
        // Change the primary organization in place; keep the others.
        let mut orgs = person["organizations"].as_array().cloned().unwrap_or_default();
        let i = orgs.iter().position(|o| o["metadata"]["primary"] == true).or((!orgs.is_empty()).then_some(0));
        match i {
            Some(i) => {
                orgs[i]["name"] = json!(new.organization);
                orgs[i]["title"] = json!(new.title);
                if new.organization.is_empty() && new.title.is_empty() && orgs[i].get("department").is_none_or(Value::is_null) {
                    orgs.remove(i);
                }
            }
            None => orgs.push(json!({ "name": new.organization, "title": new.title })),
        }
        body["organizations"] = Value::Array(orgs);
        fields.push("organizations");
    }
    if old.birthday != new.birthday {
        body["birthdays"] = ours["birthdays"].clone();
        fields.push("birthdays");
    }
    if old.notes != new.notes {
        body["biographies"] = ours["biographies"].clone();
        fields.push("biographies");
    }
    (body, fields)
}

/// Whether a contact id has the shape Google gives them, `people/<id>`, so it
/// can't steer the URL we build from it to another API path.
fn is_resource_name(rn: &str) -> bool {
    rn.strip_prefix("people/").is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}

/// [`is_resource_name`] as an error, for ids about to go into a URL.
fn resource_name(rn: &str) -> Result<&str> {
    if !is_resource_name(rn) {
        anyhow::bail!("not a Google contact id: {}", rn.chars().take(80).collect::<String>());
    }
    Ok(rn)
}

fn card(p: &Value) -> Option<(String, Option<String>, String)> {
    let rn = p["resourceName"].as_str().filter(|r| is_resource_name(r))?.to_string();
    let vcard = satchel_ical::vcard::build(None, &rn, &person_to_fields(p));
    Some((rn, p["etag"].as_str().map(String::from), vcard))
}

/// 403 for a token without the contacts scope.
fn scope_error(e: anyhow::Error) -> anyhow::Error {
    let s = e.to_string();
    if s.contains("HTTP 403") && (s.contains("insufficient") || s.contains("SCOPE") || s.contains("scope")) {
        return anyhow::Error::new(ContactsScopeMissing);
    }
    e
}

fn conflict_error(e: anyhow::Error) -> anyhow::Error {
    let s = e.to_string();
    if s.contains("FAILED_PRECONDITION") || s.contains("etag") {
        return anyhow::Error::new(ApiError::new(ErrorCode::Conflict, "the contact changed in Google Contacts"));
    }
    e
}

pub struct GoogleContactsProvider {
    api: ApiClient,
}

impl GoogleContactsProvider {
    pub fn new(creds: CredentialHandle) -> Result<Self> {
        Ok(Self { api: crate::google::api_client(creds)? })
    }

    async fn sync_all(&self, ctx: &SyncCtx, local: &str, token: Option<String>) -> Result<()> {
        let mut page: Option<String> = None;
        let mut next_sync = None;
        let mut seen = HashSet::new();
        loop {
            let mut url = format!("{PEOPLE}/people/me/connections?personFields={FIELDS}&pageSize=1000&requestSyncToken=true");
            if let Some(t) = &token {
                url.push_str(&format!("&syncToken={}", crate::net::enc(t)));
            }
            if let Some(p) = &page {
                url.push_str(&format!("&pageToken={}", crate::net::enc(p)));
            }
            let v = self.api.json(|h, t| h.get(&url).bearer_auth(t), "list contacts").await.map_err(scope_error)?;
            for p in v["connections"].as_array().cloned().unwrap_or_default() {
                let Some(rn) = p["resourceName"].as_str().filter(|r| is_resource_name(r)) else { continue };
                seen.insert(rn.to_string());
                if p["metadata"]["deleted"] == true {
                    ctx.delete_card_remote(local, rn)?;
                } else if let Some((rn, etag, vcard)) = card(&p) {
                    ctx.put_card(local, &rn, etag.as_deref(), &vcard)?;
                }
            }
            page = v["nextPageToken"].as_str().map(String::from);
            if let Some(t) = v["nextSyncToken"].as_str() {
                next_sync = Some(t.to_string());
            }
            if page.is_none() {
                break;
            }
        }
        if token.is_none() {
            // A full listing doesn't report deletions.
            for gone in ctx.store.card_remote_ids(local)?.into_keys().filter(|k| !seen.contains(k)) {
                ctx.delete_card_remote(local, &gone)?;
            }
        }
        if let Some(t) = next_sync {
            ctx.store.set_address_book_token(local, Some(&t))?;
        }
        Ok(())
    }
}

#[async_trait]
impl ContactsProvider for GoogleContactsProvider {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let map = ctx.set_address_books(&[RemoteAddressBook { remote_id: BOOK.into(), name: "Google Contacts".into(), read_only: false }])?;
        let local = &map[BOOK];
        let token = ctx.store.get_address_book(local)?.and_then(|b| b.sync_token);
        match self.sync_all(ctx, local, token.clone()).await {
            // Expired sync tokens: start over.
            Err(e) if token.is_some() && e.downcast_ref::<ContactsScopeMissing>().is_none() => {
                tracing::debug!("people sync token rejected ({e:#}); full resync");
                ctx.store.set_address_book_token(local, None)?;
                self.sync_all(ctx, local, None).await
            }
            r => r,
        }
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ContactOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            ContactOp::Put { card_remote_id, etag, vcard, .. } => {
                let fields = satchel_ical::vcard::parse(vcard).map(|c| c.fields).context("invalid vCard")?;
                let v = match card_remote_id {
                    None => {
                        let body = fields_to_person(&fields);
                        let url = format!("{PEOPLE}/people:createContact?personFields={FIELDS}");
                        self.api.json(|h, t| h.post(&url).bearer_auth(t).json(&body), "create contact").await.map_err(scope_error)?
                    }
                    Some(rn) => {
                        let rn = resource_name(rn)?;
                        let url = format!("{PEOPLE}/{rn}?personFields={UPDATE_FIELDS},metadata");
                        let person = self.api.json(|h, t| h.get(&url).bearer_auth(t), "get contact").await?;
                        if etag.is_some() && person["etag"].as_str() != etag.as_deref() {
                            return Err(conflict_error(anyhow::anyhow!("etag changed")));
                        }
                        let (body, changed) = update_body(&person, &fields);
                        if changed.is_empty() {
                            person
                        } else {
                            let url = format!("{PEOPLE}/{rn}:updateContact?updatePersonFields={}&personFields={FIELDS}", changed.join(","));
                            self.api.json(|h, t| h.patch(&url).bearer_auth(t).json(&body), "update contact").await.map_err(conflict_error)?
                        }
                    }
                };
                if let Some((rn, etag, vcard)) = card(&v) {
                    out.card_remote_id = Some(rn);
                    out.card_etag = etag;
                    out.card_vcard = Some(vcard);
                }
            }
            ContactOp::Delete { card_remote_id, .. } => {
                let url = format!("{PEOPLE}/{}:deleteContact", resource_name(card_remote_id)?);
                match self.api.send(|h, t| h.delete(&url).bearer_auth(t), "delete contact").await {
                    Ok(_) => {}
                    Err(e) if e.to_string().contains("HTTP 404") => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(out)
    }

    async fn fetch(&self, _ctx: &SyncCtx, _book: &str, card_remote_id: &str) -> Result<Option<(Option<String>, String)>> {
        let url = format!("{PEOPLE}/{}?personFields={FIELDS}", resource_name(card_remote_id)?);
        match self.api.json(|h, t| h.get(&url).bearer_auth(t), "get contact").await {
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
    fn resource_names_are_checked() {
        for ok in ["people/c1", "people/c7034_x-Y"] {
            assert!(is_resource_name(ok), "{ok}");
        }
        for bad in ["people/", "c1", "people/c1/../../v1/x", "people/c1:deleteContact", "people/c1?x=1", "people/c 1", "otherContacts/c1", "people/c1#"] {
            assert!(!is_resource_name(bad), "{bad}");
        }
        assert!(card(&json!({ "resourceName": "people/../x" })).is_none());
    }

    #[test]
    fn person_round_trip() {
        let p = json!({
            "resourceName": "people/c1", "etag": "%E1",
            "names": [{ "displayName": "Ann Lee", "metadata": { "primary": true } }],
            "emailAddresses": [{ "value": "ann@x.test", "type": "work" }, { "value": "ann@home.test" }],
            "phoneNumbers": [{ "value": "+1 555 0100", "type": "mobile" }],
            "organizations": [{ "name": "Acme", "title": "CTO" }],
            "birthdays": [{ "date": { "month": 4, "day": 2 } }],
            "biographies": [{ "value": "Met in Lisbon" }]
        });
        let f = person_to_fields(&p);
        assert_eq!(f.name, "Ann Lee");
        assert_eq!(f.emails[0].label.as_deref(), Some("work"));
        assert_eq!(f.phones[0].label.as_deref(), Some("cell"));
        assert_eq!(f.birthday.as_deref(), Some("--04-02"));
        let back = fields_to_person(&f);
        assert_eq!(back["phoneNumbers"][0]["type"], "mobile");
        assert_eq!(back["birthdays"][0]["date"]["day"], 2);
        assert!(back["birthdays"][0]["date"].get("year").is_none());
        // An edit to one field sends only that field; extra organizations
        // and the structured name stay as they are.
        let mut p2 = p.clone();
        p2["names"][0]["givenName"] = json!("Ann");
        p2["names"][0]["middleName"] = json!("Q");
        p2["organizations"] = json!([{ "name": "Acme", "title": "CTO", "department": "R&D", "metadata": { "primary": true } }, { "name": "Board", "title": "Member" }]);
        let mut edited = person_to_fields(&p2);
        edited.title = "CEO".into();
        let (body, changed) = update_body(&p2, &edited);
        assert_eq!(changed, vec!["organizations"]);
        assert!(body.get("names").is_none());
        assert_eq!(body["organizations"][0]["department"], "R&D");
        assert_eq!(body["organizations"][0]["title"], "CEO");
        assert_eq!(body["organizations"][1]["name"], "Board");
        assert!(update_body(&p2, &person_to_fields(&p2)).1.is_empty());
        let (rn, etag, vcard) = card(&p).unwrap();
        assert_eq!((rn.as_str(), etag.as_deref()), ("people/c1", Some("%E1")));
        assert_eq!(satchel_ical::vcard::parse(&vcard).unwrap().fields, f);
    }
}
