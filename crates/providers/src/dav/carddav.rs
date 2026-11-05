//! CardDAV contacts provider (RFC 6352, sync via RFC 6578 with an ETag fallback).

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, ContactOp, ContactsProvider, CredentialHandle, RemoteAddressBook, SyncCtx};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::dav::{DavClient, parse_multistatus, xml_escape};

pub struct CardDavProvider {
    dav: DavClient,
    home: Mutex<Option<String>>,
}

impl CardDavProvider {
    /// `base` is a CardDAV URL, or any URL on a server that reports an
    /// addressbook-home-set for the user (often the CalDAV URL works too).
    pub fn new(base: String, username: String, allow_invalid: bool, creds: CredentialHandle) -> Result<Self> {
        Ok(Self { dav: DavClient::new(base, username, allow_invalid, creds, "CardDAV")?, home: Mutex::new(None) })
    }

    async fn home(&self) -> Result<String> {
        if let Some(h) = self.home.lock().await.clone() {
            return Ok(h);
        }
        let prop = "<card:addressbook-home-set/>";
        let mut home = self.dav.home(prop, "addressbook-home-set").await.ok().flatten();
        if home.is_none() {
            // RFC 6764 bootstrapping.
            let wk = self.dav.url("/.well-known/carddav")?;
            if let Ok(r) = self.dav.propfind(&wk, "0", &format!("<d:current-user-principal/>{prop}")).await {
                home = r.iter().find_map(|x| x.props.get("addressbook-home-set").filter(|s| !s.is_empty()).cloned());
                if home.is_none()
                    && let Some(p) = r.iter().find_map(|x| x.props.get("current-user-principal").filter(|s| !s.is_empty()).cloned())
                {
                    let r2 = self.dav.propfind(&self.dav.url(&p)?, "0", prop).await?;
                    home = r2.iter().find_map(|x| x.props.get("addressbook-home-set").filter(|s| !s.is_empty()).cloned());
                }
                home = home.map(|h| self.dav.url(&h)).transpose()?;
            }
        }
        let home = home.context("server did not report an address book home (set a CardDAV URL for this account)")?;
        *self.home.lock().await = Some(home.clone());
        Ok(home)
    }

    async fn books(&self) -> Result<Vec<RemoteAddressBook>> {
        let home = self.home().await?;
        let r = self.dav.propfind(&home, "1", "<d:resourcetype/><d:displayname/><d:current-user-privilege-set/>").await?;
        Ok(r.into_iter()
            .filter(|x| x.resource_types.iter().any(|t| t == "addressbook"))
            .map(|x| RemoteAddressBook {
                name: x.props.get("displayname").filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| x.href.trim_end_matches('/').rsplit('/').next().unwrap_or("Contacts").to_string()),
                read_only: !x.privileges.is_empty() && !x.privileges.iter().any(|p| p == "write" || p == "write-content" || p == "all" || p == "bind"),
                remote_id: x.href,
            })
            .collect())
    }

    async fn multiget(&self, book_url: &str, hrefs: &[String]) -> Result<Vec<(String, Option<String>, String)>> {
        let mut out = vec![];
        for chunk in hrefs.chunks(100) {
            let refs: String = chunk.iter().map(|h| format!("<d:href>{}</d:href>", xml_escape(h))).collect();
            let body = format!(r#"<?xml version="1.0" encoding="utf-8"?><card:addressbook-multiget xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav"><d:prop><d:getetag/><card:address-data/></d:prop>{refs}</card:addressbook-multiget>"#);
            let (st, _, text) = self.dav.dav("REPORT", book_url, Some("1"), Some(body), &[]).await?;
            if st != 207 {
                return Err(anyhow!("addressbook-multiget HTTP {st}"));
            }
            for x in parse_multistatus(&text)?.0 {
                if let Some(data) = x.props.get("address-data").filter(|d| !d.is_empty()) {
                    out.push((x.href.clone(), x.props.get("getetag").cloned(), data.clone()));
                }
            }
        }
        Ok(out)
    }

    async fn sync_book(&self, ctx: &SyncCtx, local: &str, href: &str, token: Option<String>) -> Result<()> {
        let url = self.dav.url(href)?;
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:sync-collection xmlns:d="DAV:"><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#,
            xml_escape(token.as_deref().unwrap_or(""))
        );
        let (st, _, text) = self.dav.dav("REPORT", &url, None, Some(body), &[]).await?;
        let known = ctx.store.card_remote_ids(local)?;
        let (changed, deleted, new_token) = if st == 207 {
            let (resps, tok) = parse_multistatus(&text)?;
            let mut changed = vec![];
            let mut deleted = vec![];
            for x in resps {
                if x.href.trim_end_matches('/') == href.trim_end_matches('/') {
                    continue;
                }
                if x.status == Some(404) {
                    deleted.push(x.href);
                } else if known.get(&x.href).is_none_or(|(_, e)| *e != x.props.get("getetag").cloned()) {
                    changed.push(x.href);
                }
            }
            (changed, deleted, tok)
        } else {
            debug!("sync-collection unavailable ({st}); comparing etags");
            let r = self.dav.propfind(&url, "1", "<d:getetag/><d:resourcetype/>").await?;
            let mut seen = std::collections::HashSet::new();
            let mut changed = vec![];
            for x in r.iter().filter(|x| !x.resource_types.iter().any(|t| t == "collection")) {
                seen.insert(x.href.clone());
                if known.get(&x.href).is_none_or(|(_, e)| *e != x.props.get("getetag").cloned()) {
                    changed.push(x.href.clone());
                }
            }
            let deleted = known.keys().filter(|k| !seen.contains(*k)).cloned().collect();
            (changed, deleted, None)
        };
        for d in &deleted {
            ctx.delete_card_remote(local, d)?;
        }
        if !changed.is_empty() {
            for (h, etag, data) in self.multiget(&url, &changed).await? {
                ctx.put_card(local, &h, etag.as_deref(), &data)?;
            }
        }
        if let Some(t) = new_token {
            ctx.store.set_address_book_token(local, Some(&t))?;
        }
        Ok(())
    }
}

#[async_trait]
impl ContactsProvider for CardDavProvider {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let books = self.books().await?;
        let map = ctx.set_address_books(&books)?;
        for b in &books {
            let Some(local) = map.get(&b.remote_id) else { continue };
            let token = ctx.store.get_address_book(local)?.and_then(|s| s.sync_token);
            if let Err(e) = self.sync_book(ctx, local, &b.remote_id, token.clone()).await {
                if token.is_none() {
                    return Err(e);
                }
                warn!(book = %b.name, "contacts sync with token failed ({e:#}); full resync");
                ctx.store.set_address_book_token(local, None)?;
                self.sync_book(ctx, local, &b.remote_id, None).await?;
            }
        }
        Ok(())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ContactOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            ContactOp::Put { book_remote_id, card_remote_id, etag, uid, vcard } => {
                let href = match card_remote_id {
                    Some(h) => h.clone(),
                    None => {
                        let safe: String = uid.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
                        format!("{}/{}.vcf", book_remote_id.trim_end_matches('/'), safe)
                    }
                };
                let mut rb = self.dav.http.put(self.dav.url(&href)?).header("Content-Type", "text/vcard; charset=utf-8").body(vcard.clone());
                rb = match (card_remote_id, etag) {
                    (None, _) => rb.header("If-None-Match", "*"),
                    (Some(_), Some(e)) => rb.header("If-Match", e.as_str()),
                    _ => rb,
                };
                let resp = self.dav.auth(rb)?.send().await?;
                let st = resp.status();
                if st.as_u16() == 412 {
                    return Err(anyhow::Error::new(ApiError::new(ErrorCode::Conflict, "the contact changed on the server")));
                }
                if !st.is_success() {
                    return Err(crate::net::http_error(st, &crate::net::error_text(resp).await, "CardDAV PUT"));
                }
                out.card_etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(String::from);
                out.card_remote_id = Some(href);
            }
            ContactOp::Delete { card_remote_id, etag, .. } => {
                let mut rb = self.dav.http.delete(self.dav.url(card_remote_id)?);
                if let Some(e) = etag {
                    rb = rb.header("If-Match", e.as_str());
                }
                let resp = self.dav.auth(rb)?.send().await?;
                let st = resp.status();
                if st.as_u16() == 412 {
                    return Err(anyhow::Error::new(ApiError::new(ErrorCode::Conflict, "the contact changed on the server")));
                }
                if !(st.is_success() || st.as_u16() == 404) {
                    return Err(crate::net::http_error(st, &crate::net::error_text(resp).await, "CardDAV DELETE"));
                }
            }
        }
        Ok(out)
    }

    async fn fetch(&self, _ctx: &SyncCtx, _book: &str, card_remote_id: &str) -> Result<Option<(Option<String>, String)>> {
        let resp = self.dav.auth(self.dav.http.get(self.dav.url(card_remote_id)?))?.send().await?;
        let st = resp.status();
        if st.as_u16() == 404 || st.as_u16() == 410 {
            return Ok(None);
        }
        if !st.is_success() {
            return Err(crate::net::http_error(st, &crate::net::error_text(resp).await, "CardDAV GET"));
        }
        let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(String::from);
        Ok(Some((etag, crate::net::text_capped(resp, crate::net::DATA_LIMIT).await?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_address_book_listing_and_data() {
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:" xmlns:B="urn:ietf:params:xml:ns:carddav">
          <D:response><D:href>/dav/card/a%40x/default/</D:href><D:propstat><D:prop>
            <D:resourcetype><D:collection/><B:addressbook/></D:resourcetype><D:displayname>Contacts</D:displayname>
          </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
          <D:response><D:href>/dav/card/a%40x/default/b.vcf</D:href><D:propstat><D:prop><D:getetag>&quot;7&quot;</D:getetag>
            <B:address-data>BEGIN:VCARD&#13;
VERSION:3.0&#13;
FN:Zoë &amp; co&#13;
END:VCARD&#13;
</B:address-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        let (r, _) = parse_multistatus(xml).unwrap();
        assert!(r[0].resource_types.contains(&"addressbook".to_string()));
        assert_eq!(r[1].props["getetag"], "\"7\"");
        let card = satchel_ical::vcard::parse(&r[1].props["address-data"]).unwrap();
        assert_eq!(card.fields.name, "Zoë & co");
    }
}
